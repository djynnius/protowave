//! Distributed folder sharing (PRD §11, FR-54..59).
//!
//! A folder becomes content-defined chunks (FastCDC → cross-version dedup,
//! FR-59) in the BLAKE3 CAS, described by a `FolderManifest` that is itself
//! stored in the CAS — the manifest hash is the share id and the read
//! capability (NFR-18). Downloads assemble files chunk by chunk; chunks the
//! local server is missing are fetched from any participant domain on the
//! wave (origin first) over the signed federation channel and verified
//! against their hash before use (FR-56..57, NFR-C9). Mirroring (FR-58)
//! pins every chunk locally so shares survive the origin going offline.
//!
//! v0 transport note: chunk exchange rides the existing signed
//! server-to-server HTTP channel rather than iroh/QUIC — the iroh stack
//! does not build on the project's pinned rustc 1.75 (see ADR-4's fallback
//! discussion). The architecture (CAS, manifests, per-chunk verification,
//! multi-source fetch) is transport-agnostic.

use std::sync::Arc;

use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use prost::Message;
use serde::Deserialize;

use protowave_core::ParticipantId;
use protowave_proto::v1 as pb;

use crate::auth::{ApiError, CurrentUser};
use crate::federation;
use crate::store::{now_ms, ShareMeta, WaveMeta};
use crate::AppState;

/// FastCDC bounds: 64 KiB min / 256 KiB avg / 1 MiB max.
const CHUNK_MIN: u32 = 64 * 1024;
const CHUNK_AVG: u32 = 256 * 1024;
const CHUNK_MAX: u32 = 1024 * 1024;

pub const MAX_UPLOAD_BYTES: usize = 512 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;

async fn require_participant(
    state: &AppState,
    wave: &str,
    user: &str,
) -> Result<WaveMeta, ApiError> {
    let meta = state
        .store
        .get_wave(wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such wave".into()))?;
    if meta.participants.iter().any(|p| p == user) {
        Ok(meta)
    } else {
        Err(ApiError(StatusCode::FORBIDDEN, "not a participant".into()))
    }
}

/// Chunk one file into the CAS, returning its chunk hashes.
fn chunk_into_cas(state: &AppState, bytes: &[u8]) -> std::io::Result<Vec<String>> {
    let mut hashes = Vec::new();
    for chunk in fastcdc::v2020::FastCDC::new(bytes, CHUNK_MIN, CHUNK_AVG, CHUNK_MAX) {
        let data = &bytes[chunk.offset..chunk.offset + chunk.length];
        hashes.push(state.cas.put(data)?);
    }
    Ok(hashes)
}

fn guess_mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "md" => "text/markdown",
        "txt" => "text/plain",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" => "text/html",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// API handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct UploadQuery {
    pub wave: String,
    pub name: String,
}

/// Multipart folder upload: each part's filename is the file's relative
/// path within the folder (the browser's webkitRelativePath).
pub async fn upload(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Query(q): Query<UploadQuery>,
    mut multipart: Multipart,
) -> Result<Json<ShareMeta>, ApiError> {
    require_participant(&state, &q.wave, &me.to_string()).await?;
    let folder_name = q.name.trim();
    if folder_name.is_empty() || folder_name.len() > 120 {
        return Err(ApiError::bad_request("folder name must be 1-120 chars"));
    }

    let mut files = Vec::new();
    let mut total_size: u64 = 0;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        let path = field.file_name().unwrap_or_default().replace('\\', "/");
        if path.is_empty() || path.contains("..") {
            return Err(ApiError::bad_request("bad file path in upload"));
        }
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("{path}: file exceeds 128 MB"),
            ));
        }
        if bytes.is_empty() {
            continue;
        }
        total_size += bytes.len() as u64;
        let chunks = chunk_into_cas(&state, &bytes)?;
        files.push(pb::FileEntry {
            mime: guess_mime(&path).to_string(),
            path,
            size: bytes.len() as u64,
            chunks,
        });
    }
    if files.is_empty() {
        return Err(ApiError::bad_request("no files in upload"));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let manifest = pb::FolderManifest {
        name: folder_name.to_string(),
        total_size,
        created_ms: now_ms(),
        files,
    };
    let manifest_bytes = manifest.encode_to_vec();
    let manifest_hash = state.cas.put(&manifest_bytes)?;

    let meta = ShareMeta {
        manifest_hash: manifest_hash.clone(),
        wave: q.wave.clone(),
        name: folder_name.to_string(),
        total_size,
        file_count: manifest.files.len() as u32,
        uploader: me.to_string(),
        origin_domain: state.domain.clone(),
        mirrored: true, // the origin holds everything by definition
        created_ms: manifest.created_ms,
    };
    state.store.put_share(&meta).await?;
    state.store.touch_wave(&q.wave, now_ms()).await?;
    tracing::info!(share = %manifest_hash, wave = %q.wave, files = meta.file_count,
                   size = total_size, by = %me, "folder shared");
    spawn_share_announce(state.clone(), meta.clone());
    Ok(Json(meta))
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub wave: String,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<ShareMeta>>, ApiError> {
    require_participant(&state, &q.wave, &me.to_string()).await?;
    Ok(Json(state.store.list_shares(&q.wave).await?))
}

/// The parsed manifest as JSON for the browse dialog.
pub async fn manifest(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Path(hash): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let share = state
        .store
        .get_share(&hash)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such share".into()))?;
    require_participant(&state, &share.wave, &me.to_string()).await?;
    let manifest = load_manifest(&state, &share).await?;
    Ok(Json(serde_json::json!({
        "name": manifest.name,
        "totalSize": manifest.total_size,
        "createdMs": manifest.created_ms,
        "files": manifest.files.iter().map(|f| serde_json::json!({
            "path": f.path,
            "size": f.size,
            "mime": f.mime,
            "chunks": f.chunks.len(),
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct FileQuery {
    pub path: String,
}

/// Assemble a file from its chunks (fetching missing ones from the
/// federation) and serve it.
pub async fn download(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Path(hash): Path<String>,
    Query(q): Query<FileQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let share = state
        .store
        .get_share(&hash)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such share".into()))?;
    let wave_meta = require_participant(&state, &share.wave, &me.to_string()).await?;
    let manifest = load_manifest(&state, &share).await?;
    let entry = manifest
        .files
        .iter()
        .find(|f| f.path == q.path)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such file in share".into()))?;

    let mut body = Vec::with_capacity(entry.size as usize);
    for (i, chunk_hash) in entry.chunks.iter().enumerate() {
        let bytes = resolve_chunk(&state, &share, &wave_meta, chunk_hash, i).await?;
        body.extend_from_slice(&bytes);
    }

    let filename = entry
        .path
        .rsplit('/')
        .next()
        .unwrap_or("file")
        .replace('"', "");
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        entry
            .mime
            .parse()
            .unwrap_or(header::HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"")
            .parse()
            .unwrap_or(header::HeaderValue::from_static("attachment")),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    Ok((headers, body))
}

/// Pin every chunk of the share locally (FR-58).
pub async fn mirror(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Path(hash): Path<String>,
) -> Result<Json<ShareMeta>, ApiError> {
    let mut share = state
        .store
        .get_share(&hash)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such share".into()))?;
    let wave_meta = require_participant(&state, &share.wave, &me.to_string()).await?;
    let manifest = load_manifest(&state, &share).await?;
    for file in &manifest.files {
        for (i, chunk_hash) in file.chunks.iter().enumerate() {
            resolve_chunk(&state, &share, &wave_meta, chunk_hash, i).await?;
        }
    }
    share.mirrored = true;
    state.store.put_share(&share).await?;
    tracing::info!(share = %hash, by = %me, "share fully mirrored");
    Ok(Json(share))
}

// ---------------------------------------------------------------------------
// Chunk & manifest resolution (local CAS → federation swarm)
// ---------------------------------------------------------------------------

async fn load_manifest(
    state: &Arc<AppState>,
    share: &ShareMeta,
) -> Result<pb::FolderManifest, ApiError> {
    let bytes = match state.cas.get(&share.manifest_hash)? {
        Some(b) => b,
        None => fetch_blob_from_peers(state, share, &share.manifest_hash, 0)
            .await
            .ok_or_else(|| {
                ApiError(
                    StatusCode::BAD_GATEWAY,
                    "manifest unavailable from any peer".into(),
                )
            })?,
    };
    pb::FolderManifest::decode(bytes.as_slice())
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn resolve_chunk(
    state: &Arc<AppState>,
    share: &ShareMeta,
    wave_meta: &WaveMeta,
    hash: &str,
    index: usize,
) -> Result<Vec<u8>, ApiError> {
    if let Some(bytes) = state.cas.get(hash)? {
        return Ok(bytes);
    }
    // Swarm order: origin first, then every other remote participant domain,
    // rotated by chunk index to spread load (FR-57).
    let mut sources = vec![share.origin_domain.clone()];
    for domain in federation::remote_domains(wave_meta, &state.domain) {
        if domain != share.origin_domain {
            sources.push(domain);
        }
    }
    sources.retain(|d| d != &state.domain);
    if sources.is_empty() {
        return Err(ApiError(StatusCode::NOT_FOUND, "chunk missing".into()));
    }
    fetch_blob_from_peers_ordered(state, share, hash, index, &sources)
        .await
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                "chunk unavailable from any peer".into(),
            )
        })
}

async fn fetch_blob_from_peers(
    state: &Arc<AppState>,
    share: &ShareMeta,
    hash: &str,
    index: usize,
) -> Option<Vec<u8>> {
    let sources = vec![share.origin_domain.clone()];
    fetch_blob_from_peers_ordered(state, share, hash, index, &sources).await
}

/// Try peers in rotated order; verify BLAKE3 before accepting (NFR-C9) and
/// persist verified bytes to the local CAS.
async fn fetch_blob_from_peers_ordered(
    state: &Arc<AppState>,
    share: &ShareMeta,
    hash: &str,
    index: usize,
    sources: &[String],
) -> Option<Vec<u8>> {
    let n = sources.len();
    for k in 0..n {
        let domain = &sources[(index + k) % n];
        match federation::fetch_blob(state, domain, hash, &share.wave).await {
            Ok(bytes) => {
                if blake3::hash(&bytes).to_hex().to_string() != hash {
                    tracing::warn!(peer = %domain, %hash, "peer served corrupt blob — rejected");
                    continue;
                }
                if let Err(e) = state.cas.put(&bytes) {
                    tracing::warn!(%e, "failed to persist fetched chunk");
                }
                return Some(bytes);
            }
            Err(e) => {
                tracing::debug!(peer = %domain, %hash, %e, "blob fetch miss");
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Federation: share announcements + blob serving
// ---------------------------------------------------------------------------

fn spawn_share_announce(state: Arc<AppState>, meta: ShareMeta) {
    tokio::spawn(async move {
        let wave_meta = match state.store.get_wave(&meta.wave).await {
            Ok(Some(m)) => m,
            _ => return,
        };
        let msg = pb::ShareAnnouncement {
            wave: meta.wave.clone(),
            manifest_hash: meta.manifest_hash.clone(),
            name: meta.name.clone(),
            total_size: meta.total_size,
            file_count: meta.file_count,
            uploader: meta.uploader.clone(),
            created_ms: meta.created_ms,
        }
        .encode_to_vec();
        for domain in federation::remote_domains(&wave_meta, &state.domain) {
            if let Err(e) = state
                .federation
                .debug_post_signed(&state.domain, &domain, SHARE_ANNOUNCE_PATH, msg.clone())
                .await
            {
                tracing::warn!(%e, peer = %domain, "share announce failed");
            }
        }
    });
}

pub const SHARE_ANNOUNCE_PATH: &str = "/federation/v0/share-announce";
pub const BLOB_PATH: &str = "/federation/v0/blob";

pub async fn handle_share_announce(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Vec<u8>, ApiError> {
    let peer = federation::verify_peer_public(&state, &headers, SHARE_ANNOUNCE_PATH, &body).await?;
    let ann = pb::ShareAnnouncement::decode(body.as_ref())
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let wave_meta = state
        .store
        .get_wave(&ann.wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "unknown wave".into()))?;
    // Sender must hold a participant on the wave; the uploader must belong
    // to the sending domain (no forged attributions).
    let uploader: Option<ParticipantId> = ann.uploader.parse().ok();
    let uploader_domain = uploader.as_ref().map(|p| p.domain().to_string());
    if !wave_meta
        .participants
        .iter()
        .filter_map(|p| p.parse::<ParticipantId>().ok())
        .any(|p| p.domain() == peer)
        || uploader_domain.as_deref() != Some(peer.as_str())
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "not a participant domain".into(),
        ));
    }
    state
        .store
        .put_share(&ShareMeta {
            manifest_hash: ann.manifest_hash.clone(),
            wave: ann.wave,
            name: ann.name,
            total_size: ann.total_size,
            file_count: ann.file_count,
            uploader: ann.uploader,
            origin_domain: peer.clone(),
            mirrored: false,
            created_ms: ann.created_ms,
        })
        .await?;
    tracing::info!(share = %ann.manifest_hash, origin = %peer, "remote share announced");
    Ok(pb::FedAck {
        ok: true,
        error: String::new(),
    }
    .encode_to_vec())
}

/// Serve a CAS blob to a federated peer (chunk or manifest). ACL: the
/// requesting domain must hold a participant on the share's wave.
pub async fn handle_blob(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Vec<u8>, ApiError> {
    let peer = federation::verify_peer_public(&state, &headers, BLOB_PATH, &body).await?;
    let req =
        pb::BlobRequest::decode(body.as_ref()).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let wave_meta = state
        .store
        .get_wave(&req.wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "unknown wave".into()))?;
    if !wave_meta
        .participants
        .iter()
        .filter_map(|p| p.parse::<ParticipantId>().ok())
        .any(|p| p.domain() == peer)
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "not a participant domain".into(),
        ));
    }
    state
        .cas
        .get(&req.hash)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "blob not held here".into()))
}
