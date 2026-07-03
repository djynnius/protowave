//! Wave agents — the Hive Mind harness (PRD §12.1, exploratory).
//!
//! An agent is a virtual participant `assistant@domain` that reads a wave,
//! retrieves supporting context (recent blips + full-text retrieval over
//! the asker's accessible waves + the wave's shared files — with
//! provenance), asks an `InferenceProvider`, and writes its answer as a
//! *blip* into the wavelet's CRDT document. That blip persists, fans out to
//! every subscriber, and federates exactly like a human's edit — the agent
//! is a first-class participant, not a bolted-on chatbot (the modern
//! successor of Wave's robots API).
//!
//! Honest scope: these containers have no GPU, so the "peer-hosted model"
//! is each node's configured provider (Gemini here). The federated-infer
//! protocol (`federation::handle_infer`) is real and lets a wave's agent
//! route to another node's model — "mixture of peers". Answer verification
//! (R11) is unsolved; answers are advisory and may be cross-checked.

use std::sync::{Arc, RwLock};
use std::{collections::HashMap, io};

use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::TokioExecutor;
use rand::RngCore;
use serde::Deserialize;
use yrs::{
    Any, Array, Doc, Map, ReadTxn, Transact, XmlElementPrelim, XmlFragment, XmlFragmentPrelim,
    XmlTextPrelim,
};

use protowave_core::WaveletName;

use crate::auth::{ApiError, CurrentUser};
use crate::store::now_ms;
use crate::AppState;

/// Local part of the wave agent's address (`assistant@domain`).
pub const AGENT_LOCAL: &str = "assistant";
const RECENT_BLIPS: usize = 12;
const RAG_HITS: usize = 4;

// ---------------------------------------------------------------------------
// Provider abstraction (FI-6) — generalizes the Translator pattern.
// ---------------------------------------------------------------------------

#[async_trait]
pub trait InferenceProvider: Send + Sync + 'static {
    async fn infer(&self, prompt: &str, context: &str) -> io::Result<String>;
    fn model(&self) -> String;
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

type HttpsClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>;

/// Reference provider: a Gemini Flash-Lite-class model (FI-1).
pub struct GeminiInference {
    key: String,
    model: String,
    client: HttpsClient,
}

impl GeminiInference {
    pub fn new(key: String, model: String) -> Self {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_http1()
            .build();
        Self {
            key,
            model,
            client: Client::builder(TokioExecutor::new()).build(https),
        }
    }
}

#[async_trait]
impl InferenceProvider for GeminiInference {
    async fn infer(&self, prompt: &str, context: &str) -> io::Result<String> {
        let full = format!(
            "You are the assistant participant in a ProtoWave collaborative wave. \
             Answer the question using the CONTEXT when relevant, and cite which \
             source you used inline. Be concise and conversational — your reply is \
             posted as a message everyone on the wave will read.\n\n\
             CONTEXT:\n{context}\n\nQUESTION:\n{prompt}"
        );
        let body = serde_json::json!({
            "contents": [{ "parts": [{ "text": full }] }],
            "generationConfig": { "temperature": 0.3 }
        })
        .to_string();
        let uri: hyper::Uri = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        )
        .parse()
        .map_err(other)?;
        let req = hyper::Request::post(uri)
            .header("content-type", "application/json")
            .header("x-goog-api-key", &self.key)
            .body(Full::new(Bytes::from(body)))
            .map_err(other)?;
        let res = self.client.request(req).await.map_err(other)?;
        let status = res.status();
        let bytes = res.into_body().collect().await.map_err(other)?.to_bytes();
        if !status.is_success() {
            return Err(other(format!("gemini: {status}")));
        }
        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(other)?;
        json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .map(|s| s.trim().to_string())
            .ok_or_else(|| other("gemini: no candidate"))
    }

    fn model(&self) -> String {
        self.model.clone()
    }
}

/// Self-hosted provider: a local Ollama server (FI-6, PRD §12.1). This is
/// the "bring your own model" path — an operator runs `ollama serve`, pulls
/// a model, points a ProtoWave node at it, and that model joins the Hive
/// Mind (advertised in `.well-known`, answerable by federated peers) with
/// no protocol changes. Plaintext HTTP: Ollama listens on localhost.
pub struct OllamaInference {
    base: String,
    model: String,
    client: Client<HttpConnector, Full<Bytes>>,
}

impl OllamaInference {
    /// `base` is the Ollama server URL, e.g. `http://127.0.0.1:11434`.
    pub fn new(base: String, model: String) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            model,
            client: Client::builder(TokioExecutor::new()).build_http(),
        }
    }
}

#[async_trait]
impl InferenceProvider for OllamaInference {
    async fn infer(&self, prompt: &str, context: &str) -> io::Result<String> {
        let full = format!(
            "You are the assistant participant in a ProtoWave collaborative wave. \
             Answer using the CONTEXT when relevant and cite the source inline. \
             Be concise — your reply is posted as a message for everyone on the \
             wave.\n\nCONTEXT:\n{context}\n\nQUESTION:\n{prompt}"
        );
        let body = serde_json::json!({
            "model": self.model,
            "prompt": full,
            "stream": false,
            "options": { "temperature": 0.3 }
        })
        .to_string();
        let uri: hyper::Uri = format!("{}/api/generate", self.base)
            .parse()
            .map_err(other)?;
        let req = hyper::Request::post(uri)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(other)?;
        let res = self.client.request(req).await.map_err(other)?;
        let status = res.status();
        let bytes = res.into_body().collect().await.map_err(other)?.to_bytes();
        if !status.is_success() {
            return Err(other(format!("ollama: {status}")));
        }
        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(other)?;
        json["response"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| other("ollama: empty response"))
    }

    fn model(&self) -> String {
        // Provenance makes clear this is a self-hosted local model.
        format!("ollama/{}", self.model)
    }
}

/// Holds this node's inference provider, if any (FI-6, swappable).
#[derive(Default)]
pub struct InferenceHub {
    provider: RwLock<Option<Arc<dyn InferenceProvider>>>,
}

impl InferenceHub {
    pub fn set(&self, p: Arc<dyn InferenceProvider>) {
        *self.provider.write().unwrap() = Some(p);
    }
    pub fn get(&self) -> Option<Arc<dyn InferenceProvider>> {
        self.provider.read().unwrap().clone()
    }
    pub fn model(&self) -> Option<String> {
        self.provider.read().unwrap().as_ref().map(|p| p.model())
    }
    pub fn available(&self) -> bool {
        self.provider.read().unwrap().is_some()
    }
}

// ---------------------------------------------------------------------------
// Per-user model pool + auto-router (PRD §12.1, FI-1)
// ---------------------------------------------------------------------------

/// Caches a constructed provider per pool model so routing doesn't rebuild
/// an HTTP client on every answer. Keyed by `base|model`; pool models are
/// Ollama endpoints (no secrets), so construction is cheap and stateless.
#[derive(Default)]
pub struct ModelPool {
    cache: RwLock<HashMap<String, Arc<dyn InferenceProvider>>>,
}

impl ModelPool {
    pub fn provider_for(&self, base: &str, model: &str) -> Arc<dyn InferenceProvider> {
        let key = format!("{base}|{model}");
        if let Some(p) = self.cache.read().unwrap().get(&key) {
            return p.clone();
        }
        let provider: Arc<dyn InferenceProvider> =
            Arc::new(OllamaInference::new(base.to_string(), model.to_string()));
        self.cache.write().unwrap().insert(key, provider.clone());
        provider
    }
}

/// A chosen model plus a human-readable provenance string for the answer.
pub struct Route {
    pub provider: Arc<dyn InferenceProvider>,
    pub provenance: String,
}

fn route_from(state: &Arc<AppState>, m: &crate::store::UserModel) -> Route {
    let owner_local = m.owner.split('@').next().unwrap_or(&m.owner);
    Route {
        provider: state.model_pool.provider_for(&m.base, &m.model),
        provenance: format!("{} · hosted by {owner_local}", m.model),
    }
}

/// Auto-route an ask to a model (FI-1). Precedence: the asker's own enabled
/// model (any scope) → an enabled model shared to the wave by a participant
/// (scope `wave`/`federation`) → this node's default provider. Returns
/// `None` only when nothing at all is configured.
pub async fn select_route(
    state: &Arc<AppState>,
    meta: &crate::store::WaveMeta,
    asker: Option<&protowave_core::ParticipantId>,
) -> Option<Route> {
    let enabled: Vec<crate::store::UserModel> = state
        .store
        .list_models()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.enabled)
        .collect();

    // 1. The asker's own model, whatever its scope (it's theirs to use).
    if let Some(me) = asker {
        let me = me.to_string();
        if let Some(m) = enabled.iter().find(|m| m.owner == me) {
            return Some(route_from(state, m));
        }
    }
    // 2. A model a wave participant has shared to the wave or the federation.
    let participants: std::collections::HashSet<&String> = meta.participants.iter().collect();
    if let Some(m) = enabled
        .iter()
        .find(|m| (m.scope == "wave" || m.scope == "federation") && participants.contains(&m.owner))
    {
        return Some(route_from(state, m));
    }
    // 3. This node's default provider (owner-configured / env).
    state.inference.get().map(|provider| {
        let provenance = provider.model();
        Route {
            provider,
            provenance,
        }
    })
}

// ---------------------------------------------------------------------------
// Server-authored blips (wire-compatible with web/src/lib/wavemodel.ts).
// ---------------------------------------------------------------------------

/// Build a yrs update that appends an agent reply blip, then return the
/// encoded diff. Matches the JS document model: blip content is an
/// XmlFragment of `<paragraph>` elements in the `blips` map, and a
/// `{id, author, ts, parent}` entry in the `manifest` array. `parent` is the
/// blip the reply threads under (e.g. the message that @mentioned the
/// agent), or `None` for a top-level post — the web client's `threadOrder`
/// only renders blips reachable from an existing parent, so a stale parent
/// id would silently orphan the reply.
pub fn agent_blip_update(base: &Doc, agent: &str, text: &str, parent: Option<&str>) -> Vec<u8> {
    let blips = base.get_or_insert_map("blips");
    let manifest = base.get_or_insert_array("manifest");
    let before = base.transact().state_vector();
    let mut raw = [0u8; 6];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let id = format!("b+{}", hex::encode(raw));
    {
        let mut txn = base.transact_mut();
        let frag = blips.insert(&mut txn, id.clone(), XmlFragmentPrelim::default());
        for (i, line) in text.split('\n').enumerate() {
            let para = frag.insert(&mut txn, i as u32, XmlElementPrelim::empty("paragraph"));
            if !line.is_empty() {
                para.insert(&mut txn, 0, XmlTextPrelim::new(line));
            }
        }
        let mut entry: HashMap<String, Any> = HashMap::new();
        entry.insert("id".into(), Any::from(id.as_str()));
        entry.insert("author".into(), Any::from(agent));
        entry.insert("ts".into(), Any::Number(now_ms() as f64));
        entry.insert(
            "parent".into(),
            match parent {
                Some(p) => Any::from(p),
                None => Any::Null,
            },
        );
        manifest.push_back(&mut txn, Any::Map(Arc::new(entry)));
    }
    base.transact().encode_diff_v1(&before)
}

// ---------------------------------------------------------------------------
// Ask orchestration (RAG + inference + reply)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AskRequest {
    pub wave: String,
    pub prompt: String,
}

/// Assemble grounding context with provenance. Retrieval is scoped to the
/// asker's accessible waves (FI-5); when there is no asker (the mention
/// worker) it is skipped rather than widened.
async fn build_context(
    state: &Arc<AppState>,
    wave: &str,
    live_text: &[(String, String)],
    asker: Option<&protowave_core::ParticipantId>,
    prompt: &str,
) -> String {
    let mut ctx = String::new();

    ctx.push_str("Recent messages on this wave:\n");
    for (_, text) in live_text.iter().rev().take(RECENT_BLIPS).rev() {
        ctx.push_str("- ");
        ctx.push_str(text);
        ctx.push('\n');
    }

    if let Some(me) = asker {
        let allowed: std::collections::HashSet<String> = state
            .store
            .list_waves_for(me)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|w| w.wave)
            .collect();
        if let Ok(hits) = state.search.query(prompt, &allowed, RAG_HITS) {
            if !hits.is_empty() {
                ctx.push_str("\nRelevant waves you can access:\n");
                for h in hits {
                    ctx.push_str(&format!("- [{}] {}\n", h.title, strip_html(&h.snippet)));
                }
            }
        }
    }

    if let Ok(shares) = state.store.list_shares(wave).await {
        if !shares.is_empty() {
            ctx.push_str("\nShared files on this wave:\n");
            for s in shares {
                ctx.push_str(&format!("- {} ({} files)\n", s.name, s.file_count));
            }
        }
    }
    ctx
}

fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Core agent turn: gather context, infer, and post the answer as a blip.
/// Returns `(answer, model)`. Shared by the explicit ask API and the
/// @mention worker; does no ACL/rate-limit checks (callers do).
pub async fn answer_wave(
    state: &Arc<AppState>,
    wave: &str,
    prompt: &str,
    asker: Option<&protowave_core::ParticipantId>,
    reply_to: Option<&str>,
) -> Result<(String, String), ApiError> {
    // Route to a model from the pool (asker's own → a wave member's shared
    // model → node default). Needs the wave's participant list.
    let meta = state
        .store
        .get_wave(wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such wave".into()))?;
    let route = select_route(state, &meta, asker).await.ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "no model configured".into(),
        )
    })?;
    let root: WaveletName = format!("{wave}/conv+root")
        .parse()
        .map_err(|_| ApiError::bad_request("bad wave id"))?;
    let live = state
        .engine
        .open_wavelet(&root)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
    let live_text = live.extract_blips();
    let context = build_context(state, wave, &live_text, asker, prompt).await;
    let answer = route
        .provider
        .infer(prompt, &context)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("inference failed: {e}")))?;

    let agent = format!("{AGENT_LOCAL}@{}", state.domain);
    let update = {
        let scratch = Doc::new();
        let (_sv, diff) = live
            .sync_state(&[])
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
        if !diff.is_empty() {
            use yrs::updates::decoder::Decode;
            scratch
                .transact_mut()
                .apply_update(yrs::Update::decode_v1(&diff).unwrap());
        }
        agent_blip_update(&scratch, &agent, &answer, reply_to)
    };
    state
        .engine
        .apply_update(&live, update.clone(), 0)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
    crate::federation::spawn_push_update(state.clone(), root, update);
    Ok((answer, route.provenance))
}

pub async fn ask(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Json(req): Json<AskRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Rate limit: inference is expensive (FI-2 spirit).
    state.limits.check(
        &me.to_string(),
        "ask",
        60,
        std::time::Duration::from_secs(3600),
    )?;
    let prompt = req.prompt.trim();
    if prompt.is_empty() || prompt.len() > 4000 {
        return Err(ApiError::bad_request("prompt must be 1-4000 chars"));
    }
    let meta = state
        .store
        .get_wave(&req.wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such wave".into()))?;
    if !meta.participants.contains(&me.to_string()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "not a participant".into()));
    }
    let (answer, model) = answer_wave(&state, &req.wave, prompt, Some(&me), None).await?;
    tracing::info!(wave = %req.wave, by = %me, %model, "agent answered (ask)");
    Ok(Json(serde_json::json!({
        "answer": answer,
        "model": model,
        "agent": format!("{AGENT_LOCAL}@{}", state.domain),
    })))
}

/// Background worker: when a *new* blip mentions `@assistant`, the agent
/// answers it inline (the Wave-robots behaviour). Heavily guarded against
/// loops and cost: agent-authored blips are skipped, each blip is answered
/// at most once, only recent blips qualify (so a restart doesn't re-answer
/// history), edits are debounced, and a per-wave rate limit applies.
pub fn spawn_mention_worker(state: Arc<AppState>) {
    let agent = format!("{AGENT_LOCAL}@{}", state.domain);
    let mention = format!("@{AGENT_LOCAL}");
    let mut rx = state.engine.change_stream();
    tokio::spawn(async move {
        let mut handled: std::collections::HashSet<String> = std::collections::HashSet::new();
        while let Some(name) = rx.recv().await {
            // Proceed if the node has a default model or the pool holds any
            // enabled one (only pay the pool lookup when there's no default).
            if !state.inference.available() {
                let has_pool = state
                    .store
                    .list_models()
                    .await
                    .map(|v| v.iter().any(|m| m.enabled))
                    .unwrap_or(false);
                if !has_pool {
                    continue;
                }
            }
            // Let the author finish typing the mention before we read it.
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            let live = match state.engine.open_wavelet(&name).await {
                Ok(l) => l,
                Err(_) => continue,
            };
            let wave = name.wave_id.to_string();
            for blip in live.blips_detailed() {
                if handled.contains(&blip.id) {
                    continue;
                }
                // Skip the agent's own blips (no loops).
                if blip.author == agent {
                    handled.insert(blip.id);
                    continue;
                }
                let recent = now_ms().saturating_sub(blip.ts) < 10 * 60 * 1000;
                if !blip.text.contains(&mention) {
                    continue;
                }
                // Mark handled up-front so a mid-answer edit can't double-fire.
                handled.insert(blip.id.clone());
                if !recent {
                    continue; // old mention seen after a restart — ignore
                }
                // Per-wave budget guard against @assistant spam.
                if state
                    .limits
                    .check(&wave, "mention", 20, std::time::Duration::from_secs(3600))
                    .is_err()
                {
                    continue;
                }
                let asker = blip.author.parse::<protowave_core::ParticipantId>().ok();
                // Thread the reply under the message that mentioned us.
                match answer_wave(&state, &wave, &blip.text, asker.as_ref(), Some(&blip.id)).await {
                    Ok((_, model)) => {
                        tracing::info!(%wave, %model, "agent answered (@mention)")
                    }
                    Err(e) => tracing::warn!(err = %e.1, %wave, "mention answer failed"),
                }
            }
        }
    });
}
