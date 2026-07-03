//! Server-to-server federation, protocol v0 (PRD §8.3,
//! docs/federation-spec.md).
//!
//! Message-level security: every s2s request is signed with the sender's
//! ed25519 server key over `"protowave-fed-v0\n" ‖ path ‖ "\n" ‖ body`;
//! receivers resolve and pin peer keys via `/.well-known/protowave` on
//! first contact (TOFU, NFR-15). Transport v0 is HTTP on each server's
//! public URL — the schema in `federation.proto` is transport-agnostic and
//! can move to gRPC without change.
//!
//! Content plane: locally-originated updates are pushed to every peer
//! domain on the wave (FR-48..49); anti-entropy state-vector sync heals
//! gaps (FR-50). Control plane: only the wave's home domain may announce
//! membership (FR-51).

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::TokioExecutor;
use prost::Message;

use protowave_core::{ParticipantId, ServerKeypair, ServerPublicKey, Signature, WaveletName};
use protowave_proto::v1 as pb;

use crate::auth::ApiError;
use crate::store::{now_ms, WaveMeta};
use crate::AppState;

pub const PROTOCOL_VERSION: u32 = 0;
const SIG_CONTEXT: &[u8] = b"protowave-fed-v0\n";
pub const PUSH_PATH: &str = "/federation/v0/push";
pub const SYNC_PATH: &str = "/federation/v0/sync";
pub const ANNOUNCE_PATH: &str = "/federation/v0/announce";

const DOMAIN_HEADER: &str = "x-protowave-domain";
const SIGNATURE_HEADER: &str = "x-protowave-signature";

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

#[derive(Clone, Default)]
pub struct FederationConfig {
    /// Base URL peers can reach us at (advertised in /.well-known).
    pub public_url: String,
    /// Static peer resolution: domain → base URL. (DNS-based .well-known
    /// discovery works when domains resolve; on a LAN this map is the
    /// source of truth.)
    pub peers: HashMap<String, String>,
}

type HttpClient = Client<HttpConnector, Full<Bytes>>;

pub struct Federation {
    pub config: FederationConfig,
    keypair: ServerKeypair,
    client: HttpClient,
}

impl Federation {
    pub fn new(config: FederationConfig, data_dir: &Path) -> io::Result<Self> {
        Ok(Self {
            config,
            keypair: load_or_create_keypair(data_dir)?,
            client: Client::builder(TokioExecutor::new()).build_http(),
        })
    }

    pub fn public_key(&self) -> ServerPublicKey {
        self.keypair.public_key()
    }

    fn peer_url(&self, domain: &str) -> Option<&str> {
        self.config.peers.get(domain).map(String::as_str)
    }

    fn sign(&self, path: &str, body: &[u8]) -> String {
        let msg = [SIG_CONTEXT, path.as_bytes(), b"\n", body].concat();
        self.keypair.sign(&msg).to_hex()
    }

    async fn post_signed(
        &self,
        our_domain: &str,
        peer_domain: &str,
        path: &str,
        body: Vec<u8>,
    ) -> io::Result<Vec<u8>> {
        let base = self
            .peer_url(peer_domain)
            .ok_or_else(|| other(format!("no peer URL for {peer_domain}")))?;
        let uri: hyper::Uri = format!("{base}{path}").parse().map_err(other)?;
        let req = hyper::Request::post(uri)
            .header("content-type", "application/x-protobuf")
            .header(DOMAIN_HEADER, our_domain)
            .header(SIGNATURE_HEADER, self.sign(path, &body))
            .body(Full::new(Bytes::from(body)))
            .map_err(other)?;
        let res = self.client.request(req).await.map_err(other)?;
        let status = res.status();
        let bytes = res.into_body().collect().await.map_err(other)?.to_bytes();
        if !status.is_success() {
            return Err(other(format!(
                "{peer_domain}{path}: {status} {}",
                String::from_utf8_lossy(&bytes)
            )));
        }
        Ok(bytes.to_vec())
    }

    /// Test hook: send a signed s2s request directly.
    #[doc(hidden)]
    pub async fn debug_post_signed(
        &self,
        our_domain: &str,
        peer_domain: &str,
        path: &str,
        body: Vec<u8>,
    ) -> io::Result<Vec<u8>> {
        self.post_signed(our_domain, peer_domain, path, body).await
    }

    /// GET a peer's /.well-known/protowave and return its public key.
    async fn fetch_peer_key(&self, peer_domain: &str) -> io::Result<String> {
        let base = self
            .peer_url(peer_domain)
            .ok_or_else(|| other(format!("no peer URL for {peer_domain}")))?;
        let uri: hyper::Uri = format!("{base}/.well-known/protowave")
            .parse()
            .map_err(other)?;
        let req = hyper::Request::get(uri)
            .body(Full::new(Bytes::new()))
            .map_err(other)?;
        let res = self.client.request(req).await.map_err(other)?;
        let bytes = res.into_body().collect().await.map_err(other)?.to_bytes();
        let info: serde_json::Value = serde_json::from_slice(&bytes).map_err(other)?;
        let domain = info["domain"].as_str().unwrap_or_default();
        let key = info["publicKey"].as_str().unwrap_or_default();
        if domain != peer_domain || key.len() != 64 {
            return Err(other(format!("bad well-known from {peer_domain}")));
        }
        Ok(key.to_string())
    }
}

fn load_or_create_keypair(data_dir: &Path) -> io::Result<ServerKeypair> {
    let path = data_dir.join("server_key");
    match std::fs::read(&path) {
        Ok(bytes) => ServerKeypair::from_secret_bytes(&bytes).map_err(other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let kp = ServerKeypair::generate();
            std::fs::create_dir_all(data_dir)?;
            std::fs::write(&path, kp.to_secret_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            tracing::info!(key = %kp.public_key().to_hex(), "generated server signing key");
            Ok(kp)
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Outbound (client side)
// ---------------------------------------------------------------------------

/// Every remote domain with a participant on this wave.
pub fn remote_domains(meta: &WaveMeta, our_domain: &str) -> Vec<String> {
    let mut out: Vec<String> = meta
        .participants
        .iter()
        .filter_map(|p| p.parse::<ParticipantId>().ok())
        .map(|p| p.domain().to_string())
        .filter(|d| d != our_domain)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Push a locally-originated update to all peers on the wave (FR-48..49).
/// Fire-and-forget per peer; anti-entropy heals delivery failures.
pub fn spawn_push_update(state: Arc<AppState>, wavelet: WaveletName, update: Vec<u8>) {
    tokio::spawn(async move {
        let meta = match state.store.get_wave(&wavelet.wave_id.to_string()).await {
            Ok(Some(meta)) => meta,
            _ => return,
        };
        let batch = pb::UpdateBatch {
            wavelet: wavelet.to_string(),
            update,
            acl_version: meta.acl_version,
        }
        .encode_to_vec();
        for domain in remote_domains(&meta, &state.domain) {
            let outcome = state
                .federation
                .post_signed(&state.domain, &domain, PUSH_PATH, batch.clone())
                .await;
            if let Err(e) = outcome {
                tracing::warn!(%e, peer = %domain, wavelet = %wavelet, "federated push failed");
            }
        }
    });
}

/// Announce the authoritative membership to all remote domains (FR-51).
/// Only meaningful when we are the wave's home server.
pub fn spawn_announce(state: Arc<AppState>, meta: WaveMeta) {
    tokio::spawn(async move {
        let msg = pb::WaveAnnouncement {
            wave: meta.wave.clone(),
            title: meta.title.clone(),
            participants: meta.participants.clone(),
            created_by: meta.created_by.clone(),
            created_ms: meta.created_ms,
            acl_version: meta.acl_version,
            translation_enabled: meta.translation_enabled,
        }
        .encode_to_vec();
        for domain in remote_domains(&meta, &state.domain) {
            let outcome = state
                .federation
                .post_signed(&state.domain, &domain, ANNOUNCE_PATH, msg.clone())
                .await;
            if let Err(e) = outcome {
                tracing::warn!(%e, peer = %domain, wave = %meta.wave, "announce failed");
            }
        }
    });
}

/// Anti-entropy pull from the wavelet's home server (FR-50): exchange state
/// vectors, apply their diff, push back what they were missing.
pub fn spawn_sync_pull(state: Arc<AppState>, wavelet: WaveletName) {
    let home = wavelet.wave_id.domain().to_string();
    if home == state.domain {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = sync_pull(&state, &wavelet, &home).await {
            tracing::warn!(%e, wavelet = %wavelet, "anti-entropy pull failed");
        }
    });
}

async fn sync_pull(state: &Arc<AppState>, wavelet: &WaveletName, home: &str) -> io::Result<()> {
    let live = state
        .engine
        .open_wavelet(wavelet)
        .await
        .map_err(|e| other(format!("{e:?}")))?;
    let (our_sv, _) = live.sync_state(&[]).map_err(|e| other(format!("{e:?}")))?;
    let req = pb::FedSyncRequest {
        wavelet: wavelet.to_string(),
        state_vector: our_sv,
    }
    .encode_to_vec();
    let res = state
        .federation
        .post_signed(&state.domain, home, SYNC_PATH, req)
        .await?;
    let res = pb::FedSyncResponse::decode(res.as_slice()).map_err(other)?;
    if !res.diff.is_empty() {
        state
            .engine
            .apply_update(&live, res.diff, 0)
            .await
            .map_err(|e| other(format!("{e:?}")))?;
    }
    // Push back what the home server is missing.
    let (_, missing) = live
        .sync_state(&res.state_vector)
        .map_err(|e| other(format!("{e:?}")))?;
    if missing.len() > 2 {
        let meta = state
            .store
            .get_wave(&wavelet.wave_id.to_string())
            .await?
            .ok_or_else(|| other("wave meta missing"))?;
        let batch = pb::UpdateBatch {
            wavelet: wavelet.to_string(),
            update: missing,
            acl_version: meta.acl_version,
        }
        .encode_to_vec();
        state
            .federation
            .post_signed(&state.domain, home, PUSH_PATH, batch)
            .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Inbound (handlers)
// ---------------------------------------------------------------------------

/// Verify an inbound s2s request; returns the authenticated peer domain.
async fn verify_peer(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    body: &[u8],
) -> Result<String, ApiError> {
    let reject = |msg: &str| ApiError(StatusCode::UNAUTHORIZED, msg.into());
    let domain = headers
        .get(DOMAIN_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| reject("missing domain header"))?
        .to_string();
    let sig_hex = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| reject("missing signature header"))?;
    let sig_bytes = hex::decode(sig_hex).map_err(|_| reject("bad signature encoding"))?;
    let signature =
        Signature::from_bytes(&sig_bytes).map_err(|_| reject("bad signature length"))?;

    // Pinned key, or TOFU fetch from the peer's well-known.
    let key_hex = match state.store.get_peer_key(&domain).await? {
        Some(k) => k,
        None => {
            let fetched = state
                .federation
                .fetch_peer_key(&domain)
                .await
                .map_err(|e| ApiError(StatusCode::UNAUTHORIZED, format!("unknown peer: {e}")))?;
            state.store.put_peer_key(&domain, &fetched).await?;
            tracing::info!(peer = %domain, "pinned new federation peer key");
            fetched
        }
    };
    let key_bytes = hex::decode(key_hex).map_err(|_| reject("corrupt pinned key"))?;
    let key = ServerPublicKey::from_bytes(&key_bytes).map_err(|_| reject("corrupt pinned key"))?;

    let msg = [SIG_CONTEXT, path.as_bytes(), b"\n", body].concat();
    key.verify(&msg, &signature)
        .map_err(|_| reject("signature verification failed"))?;
    Ok(domain)
}

/// Public wrapper so other modules (shares) can authenticate s2s requests.
pub async fn verify_peer_public(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    body: &[u8],
) -> Result<String, ApiError> {
    verify_peer(state, headers, path, body).await
}

/// Fetch a CAS blob (chunk/manifest) from a peer over the signed channel.
/// The caller verifies the returned bytes against the requested hash.
pub async fn fetch_blob(
    state: &Arc<AppState>,
    peer_domain: &str,
    hash: &str,
    wave: &str,
) -> io::Result<Vec<u8>> {
    let req = pb::BlobRequest {
        hash: hash.to_string(),
        wave: wave.to_string(),
    }
    .encode_to_vec();
    state
        .federation
        .post_signed(&state.domain, peer_domain, crate::shares::BLOB_PATH, req)
        .await
}

fn domain_is_participant(meta: &WaveMeta, domain: &str) -> bool {
    meta.participants
        .iter()
        .filter_map(|p| p.parse::<ParticipantId>().ok())
        .any(|p| p.domain() == domain)
}

fn ack() -> Vec<u8> {
    pb::FedAck {
        ok: true,
        error: String::new(),
    }
    .encode_to_vec()
}

pub async fn well_known(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    // Capability manifests (FI-1): every federation-scoped pool model, so a
    // peer's router can target one by id. Only the id/model/owner are exposed
    // — never the base URL (that endpoint is the host's private business).
    let models: Vec<serde_json::Value> = state
        .store
        .list_models()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.enabled && m.scope == "federation")
        .map(|m| {
            let owner_local = m.owner.split('@').next().unwrap_or(&m.owner).to_string();
            serde_json::json!({
                "id": m.id,
                "label": m.label,
                "model": m.model,
                "owner": owner_local,
            })
        })
        .collect();
    Json(serde_json::json!({
        "domain": state.domain,
        "publicUrl": state.federation.config.public_url,
        "publicKey": state.federation.public_key().to_hex(),
        "protocolVersion": PROTOCOL_VERSION,
        // Advertised inference capability (FI-1) — null when this node hosts
        // no default model. `models` lists federation-shared pool entries.
        "inferenceModel": state.inference.model(),
        "models": models,
    }))
}

pub const INFER_PATH: &str = "/federation/v0/infer";

/// Fetch an inference from a peer node's model (FI, mixture-of-peers).
pub async fn peer_infer(
    state: &Arc<AppState>,
    peer_domain: &str,
    wave: &str,
    prompt: &str,
    context: &str,
    model: &str,
) -> io::Result<(String, String)> {
    let req = pb::InferRequest {
        wave: wave.to_string(),
        prompt: prompt.to_string(),
        context: context.to_string(),
        model: model.to_string(),
    }
    .encode_to_vec();
    let bytes = state
        .federation
        .post_signed(&state.domain, peer_domain, INFER_PATH, req)
        .await?;
    let res = pb::InferResponse::decode(bytes.as_slice()).map_err(other)?;
    Ok((res.text, res.model))
}

/// Serve an inference to a peer whose domain participates in the wave
/// (FI-4/5). Answers are advisory; verification (R11) is the caller's.
pub async fn handle_infer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Vec<u8>, ApiError> {
    let peer = verify_peer(&state, &headers, INFER_PATH, &body).await?;
    let req = pb::InferRequest::decode(body.as_ref())
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let meta = state
        .store
        .get_wave(&req.wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "unknown wave".into()))?;
    if !domain_is_participant(&meta, &peer) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "not a participant domain".into(),
        ));
    }
    // Resolve the target: a specific pool model the peer named (must be
    // federation-scoped and enabled), otherwise this node's default provider.
    let (provider, model_id) = if req.model.is_empty() {
        let p = state
            .inference
            .get()
            .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "no model here".into()))?;
        let id = p.model();
        (p, id)
    } else {
        let m = state
            .store
            .get_model(&req.model)
            .await?
            .filter(|m| m.enabled && m.scope == "federation")
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such federation model".into()))?;
        let owner_local = m.owner.split('@').next().unwrap_or(&m.owner).to_string();
        (
            state.model_pool.provider_for(&m.base, &m.model),
            format!("{} · hosted by {owner_local}", m.model),
        )
    };
    let text = provider
        .infer(&req.prompt, &req.context)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("inference: {e}")))?;
    Ok(pb::InferResponse {
        text,
        model: model_id,
    }
    .encode_to_vec())
}

pub async fn handle_push(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Vec<u8>, ApiError> {
    let peer = verify_peer(&state, &headers, PUSH_PATH, &body).await?;
    let batch =
        pb::UpdateBatch::decode(body.as_ref()).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let name: WaveletName = batch
        .wavelet
        .parse()
        .map_err(|_| ApiError::bad_request("bad wavelet name"))?;
    let meta = state
        .store
        .get_wave(&name.wave_id.to_string())
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "unknown wave".into()))?;
    // ACL: the sending domain must currently hold a participant (FR-51's
    // removed-participant rule in its v0 form).
    if !domain_is_participant(&meta, &peer) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "not a participant domain".into(),
        ));
    }

    let live = state
        .engine
        .open_wavelet(&name)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
    state
        .engine
        .apply_update(&live, batch.update, 0)
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:?}")))?;
    tracing::debug!(peer = %peer, wavelet = %batch.wavelet, "applied federated update");
    Ok(ack())
}

pub async fn handle_sync(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Vec<u8>, ApiError> {
    let peer = verify_peer(&state, &headers, SYNC_PATH, &body).await?;
    let req = pb::FedSyncRequest::decode(body.as_ref())
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let name: WaveletName = req
        .wavelet
        .parse()
        .map_err(|_| ApiError::bad_request("bad wavelet name"))?;
    let meta = state
        .store
        .get_wave(&name.wave_id.to_string())
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "unknown wave".into()))?;
    if !domain_is_participant(&meta, &peer) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "not a participant domain".into(),
        ));
    }
    let live = state
        .engine
        .open_wavelet(&name)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
    let (sv, diff) = live
        .sync_state(&req.state_vector)
        .map_err(|_| ApiError::bad_request("bad state vector"))?;
    Ok(pb::FedSyncResponse {
        state_vector: sv,
        diff,
    }
    .encode_to_vec())
}

pub async fn handle_announce(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Vec<u8>, ApiError> {
    let peer = verify_peer(&state, &headers, ANNOUNCE_PATH, &body).await?;
    let ann = pb::WaveAnnouncement::decode(body.as_ref())
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let wave: protowave_core::WaveId = ann
        .wave
        .parse()
        .map_err(|_| ApiError::bad_request("bad wave id"))?;
    // Control plane authority: only the wave's home domain may announce.
    if wave.domain() != peer {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "announcement not from home server".into(),
        ));
    }
    // Never regress the ACL version (stale/replayed announcements).
    if let Some(existing) = state.store.get_wave(&ann.wave).await? {
        if ann.acl_version < existing.acl_version {
            return Err(ApiError(StatusCode::CONFLICT, "stale acl version".into()));
        }
    }
    let meta = WaveMeta {
        wave: ann.wave.clone(),
        title: ann.title,
        participants: ann.participants,
        created_by: ann.created_by,
        created_ms: ann.created_ms,
        last_activity_ms: now_ms(),
        acl_version: ann.acl_version,
        translation_enabled: ann.translation_enabled,
        archived: false,
    };
    state.store.put_wave(&meta).await?;
    tracing::info!(wave = %ann.wave, home = %peer, "wave announced by home server");

    // Warm the replica: pull the root wavelet's content from home.
    let root: Result<WaveletName, _> = format!("{}/{}", ann.wave, crate::api::ROOT_WAVELET).parse();
    if let Ok(root) = root {
        spawn_sync_pull(state.clone(), root);
    }
    Ok(ack())
}
