//! Wave lifecycle REST API (FR-5..6, FR-28).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use protowave_core::{ParticipantId, WaveId};

use crate::auth::{ApiError, CurrentUser};
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
}

impl From<WaveMeta> for WaveDigest {
    fn from(meta: WaveMeta) -> Self {
        let root_wavelet = format!("{}/{ROOT_WAVELET}", meta.wave);
        Self {
            wave: meta.wave,
            title: meta.title,
            participants: meta.participants,
            root_wavelet,
            created_by: meta.created_by,
            last_activity_ms: meta.last_activity_ms,
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
    };
    state.store.put_wave(&meta)?;
    tracing::info!(wave = %meta.wave, by = %me, "wave created");
    Ok((StatusCode::CREATED, Json(meta.into())))
}

pub async fn list_waves(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
) -> Result<Json<Vec<WaveDigest>>, ApiError> {
    let waves = state.store.list_waves_for(&me)?;
    Ok(Json(waves.into_iter().map(WaveDigest::from).collect()))
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
    let added: ParticipantId = req
        .participant
        .parse()
        .map_err(|e| ApiError::bad_request(format!("invalid participant: {e}")))?;
    if state.store.get_account(&added)?.is_none() {
        return Err(ApiError::bad_request("no such account on this server"));
    }

    let mut meta = state
        .store
        .get_wave(&req.wave)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such wave".into()))?;
    if !meta.participants.contains(&me.to_string()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "not a participant".into()));
    }
    let addr = added.to_string();
    if !meta.participants.contains(&addr) {
        meta.participants.push(addr);
        meta.last_activity_ms = now_ms();
        state.store.put_wave(&meta)?;
        tracing::info!(wave = %meta.wave, participant = %added, by = %me, "participant added");
    }
    Ok(Json(meta.into()))
}
