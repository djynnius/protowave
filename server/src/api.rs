//! Wave lifecycle REST API (FR-5..6, FR-8, FR-25..26, FR-28..29).

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use protowave_core::{ParticipantId, WaveId, WaveletName};

use crate::auth::{ApiError, CurrentUser};
use crate::search::SearchHit;
use crate::store::{now_ms, WaveMeta};
use crate::AppState;

/// The root conversation wavelet id within a wave (legacy Wave used
/// `conv+root` for exactly this).
pub const ROOT_WAVELET: &str = "conv+root";

#[derive(Serialize)]
pub struct WaveDigest {
    pub wave: String,
    pub title: String,
    pub participants: Vec<String>,
    #[serde(rename = "rootWavelet")]
    pub root_wavelet: String,
    #[serde(rename = "createdBy")]
    pub created_by: String,
    #[serde(rename = "lastActivityMs")]
    pub last_activity_ms: u64,
    pub unread: bool,
    #[serde(rename = "translationEnabled")]
    pub translation_enabled: bool,
}

impl WaveDigest {
    fn new(meta: WaveMeta, read_mark: u64) -> Self {
        let root_wavelet = format!("{}/{ROOT_WAVELET}", meta.wave);
        Self {
            unread: meta.last_activity_ms > read_mark,
            wave: meta.wave,
            title: meta.title,
            participants: meta.participants,
            root_wavelet,
            created_by: meta.created_by,
            last_activity_ms: meta.last_activity_ms,
            translation_enabled: meta.translation_enabled,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateWaveRequest {
    pub title: String,
}

pub async fn create_wave(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Json(req): Json<CreateWaveRequest>,
) -> Result<(StatusCode, Json<WaveDigest>), ApiError> {
    state.limits.check(
        &me.to_string(),
        "create-wave",
        30,
        std::time::Duration::from_secs(3600),
    )?;
    let title = req.title.trim();
    if title.is_empty() || title.len() > 200 {
        return Err(ApiError::bad_request("title must be 1-200 characters"));
    }
    let mut raw = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let wave = WaveId::new(&state.domain, &format!("w+{}", hex::encode(raw)))
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let now = now_ms();
    let meta = WaveMeta {
        wave: wave.to_string(),
        title: title.to_string(),
        participants: vec![me.to_string()],
        created_by: me.to_string(),
        created_ms: now,
        last_activity_ms: now,
        acl_version: 1,
        translation_enabled: false,
    };
    state.store.put_wave(&meta).await?;
    tracing::info!(wave = %meta.wave, by = %me, "wave created");
    Ok((StatusCode::CREATED, Json(WaveDigest::new(meta, now))))
}

pub async fn list_waves(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
) -> Result<Json<Vec<WaveDigest>>, ApiError> {
    let waves = state.store.list_waves_for(&me).await?;
    let marks = state.store.read_marks(&me).await?;
    Ok(Json(
        waves
            .into_iter()
            .map(|meta| {
                let mark = marks.get(&meta.wave).copied().unwrap_or(0);
                WaveDigest::new(meta, mark)
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct AddParticipantRequest {
    pub wave: String,
    pub participant: String,
}

pub async fn add_participant(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Json(req): Json<AddParticipantRequest>,
) -> Result<Json<WaveDigest>, ApiError> {
    state.limits.check(
        &me.to_string(),
        "add-participant",
        120,
        std::time::Duration::from_secs(3600),
    )?;
    let added: ParticipantId = req
        .participant
        .parse()
        .map_err(|e| ApiError::bad_request(format!("invalid participant: {e}")))?;
    // Local participants must exist; remote ones are validated by their own
    // server when they authenticate there (FR-48).
    if added.domain() == state.domain && state.store.get_account(&added).await?.is_none() {
        return Err(ApiError::bad_request("no such account on this server"));
    }

    let mut meta = state
        .store
        .get_wave(&req.wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such wave".into()))?;
    if !meta.participants.contains(&me.to_string()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "not a participant".into()));
    }
    // Membership is home-server authority (FR-51): remote servers request
    // changes from the home server rather than editing replicas.
    let home = meta.wave.split('/').next().unwrap_or_default().to_string();
    if home != state.domain {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            format!("membership is managed by the wave's home server ({home})"),
        ));
    }
    let addr = added.to_string();
    if !meta.participants.contains(&addr) {
        meta.participants.push(addr);
        meta.last_activity_ms = now_ms();
        meta.acl_version += 1;
        state.store.put_wave(&meta).await?;
        tracing::info!(wave = %meta.wave, participant = %added, by = %me, "participant added");
        // Distribute the authoritative membership to peer servers.
        crate::federation::spawn_announce(state.clone(), meta.clone());
    }
    Ok(Json(WaveDigest::new(meta, u64::MAX)))
}

// ---- translation opt-in (FR-40) ----

#[derive(Deserialize)]
pub struct SetTranslationRequest {
    pub wave: String,
    pub enabled: bool,
}

pub async fn set_translation(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Json(req): Json<SetTranslationRequest>,
) -> Result<Json<WaveDigest>, ApiError> {
    let mut meta = state
        .store
        .get_wave(&req.wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such wave".into()))?;
    if !meta.participants.contains(&me.to_string()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "not a participant".into()));
    }
    let home = meta.wave.split('/').next().unwrap_or_default().to_string();
    if home != state.domain {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            format!("translation policy is managed by the wave's home server ({home})"),
        ));
    }
    if meta.translation_enabled != req.enabled {
        meta.translation_enabled = req.enabled;
        meta.acl_version += 1;
        state.store.put_wave(&meta).await?;
        tracing::info!(wave = %meta.wave, enabled = req.enabled, by = %me, "translation toggled");
        crate::federation::spawn_announce(state.clone(), meta.clone());
    }
    Ok(Json(WaveDigest::new(meta, u64::MAX)))
}

// ---- read marks (FR-8) ----

#[derive(Deserialize)]
pub struct MarkReadRequest {
    pub wave: String,
}

pub async fn mark_read(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Json(req): Json<MarkReadRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.store.set_read_mark(&me, &req.wave, now_ms()).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---- playback (FR-25..26) ----

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub wavelet: String,
}

/// The full update log as length-prefixed binary frames (the same format as
/// the on-disk log). The client replays 0..k to materialize any version.
pub async fn history(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Query(q): Query<HistoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let name: WaveletName = q
        .wavelet
        .parse()
        .map_err(|_| ApiError::bad_request("bad wavelet name"))?;
    let meta = state
        .store
        .get_wave(&name.wave_id.to_string())
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such wave".into()))?;
    if !meta.participants.contains(&me.to_string()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "not a participant".into()));
    }

    let updates = state.store.read_all_updates(&name).await?;
    let mut body = Vec::new();
    for update in &updates {
        body.extend_from_slice(&(update.len() as u32).to_le_bytes());
        body.extend_from_slice(update);
    }
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], body))
}

// ---- search (FR-29) ----

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<SearchHit>>, ApiError> {
    if query.q.trim().is_empty() {
        return Ok(Json(Vec::new()));
    }
    let allowed: HashSet<String> = state
        .store
        .list_waves_for(&me)
        .await?
        .into_iter()
        .map(|w| w.wave)
        .collect();
    let hits = state.search.query(&query.q, &allowed, 20)?;
    Ok(Json(hits))
}
