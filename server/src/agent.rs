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
const ROOT_BLIP: &str = "b+root";
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
// Server-authored blips (wire-compatible with web/src/lib/wavemodel.ts).
// ---------------------------------------------------------------------------

/// Build a yrs update that appends an agent reply blip under the root, then
/// return the encoded diff. Matches the JS document model: blip content is
/// an XmlFragment of `<paragraph>` elements in the `blips` map, and a
/// `{id, author, ts, parent}` entry in the `manifest` array.
pub fn agent_blip_update(base: &Doc, agent: &str, text: &str) -> Vec<u8> {
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
        entry.insert("parent".into(), Any::from(ROOT_BLIP));
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

/// Assemble grounding context with provenance (FI-5: only the asker's own
/// accessible content is ever retrieved).
async fn build_context(
    state: &Arc<AppState>,
    wave: &str,
    live_text: &[(String, String)],
    me: &protowave_core::ParticipantId,
    prompt: &str,
) -> String {
    let mut ctx = String::new();

    // Recent conversation.
    ctx.push_str("Recent messages on this wave:\n");
    for (_, text) in live_text.iter().rev().take(RECENT_BLIPS).rev() {
        ctx.push_str("- ");
        ctx.push_str(text);
        ctx.push('\n');
    }

    // Retrieval across the asker's waves (cited by wave title).
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

    // Shared files on this wave (cited by name).
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
    let provider = state.inference.get().ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "no model configured".into(),
        )
    })?;

    let root: WaveletName = format!("{}/conv+root", req.wave)
        .parse()
        .map_err(|_| ApiError::bad_request("bad wave id"))?;
    let live = state
        .engine
        .open_wavelet(&root)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
    let live_text = live.extract_blips();
    let context = build_context(&state, &req.wave, &live_text, &me, prompt).await;

    let answer = provider
        .infer(prompt, &context)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("inference failed: {e}")))?;

    // Write the agent's reply as a blip through the engine (persists, fans
    // out to subscribers, federates).
    let agent = format!("{AGENT_LOCAL}@{}", state.domain);
    let update = {
        let scratch = Doc::new();
        // Materialize current state so the new blip stacks onto it.
        let (_sv, diff) = live
            .sync_state(&[])
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
        if !diff.is_empty() {
            use yrs::updates::decoder::Decode;
            scratch
                .transact_mut()
                .apply_update(yrs::Update::decode_v1(&diff).unwrap());
        }
        agent_blip_update(&scratch, &agent, &answer)
    };
    state
        .engine
        .apply_update(&live, update.clone(), 0)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
    // Fan the agent's blip out to federated peers, like any local edit.
    crate::federation::spawn_push_update(state.clone(), root, update);
    tracing::info!(wave = %req.wave, by = %me, model = %provider.model(), "agent answered");

    Ok(Json(serde_json::json!({
        "answer": answer,
        "model": provider.model(),
        "agent": agent,
    })))
}
