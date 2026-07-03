//! Server settings (FR-61). The owner — the first account registered on
//! this server — can change the hosted inference model at runtime (the
//! "select architecture + model" flow) and it persists across restarts.
//! Per-user model contributions to the federated pool (multi-model routing)
//! are future work (PRD §12.1, FI-1).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use protowave_core::ParticipantId;

use crate::agent::{GeminiInference, OllamaInference};
use crate::auth::{ApiError, CurrentUser};
use crate::AppState;

pub const OWNER_KEY: &str = "owner";
const PROVIDER_KEY: &str = "inference_provider";
const BASE_KEY: &str = "inference_base";
const MODEL_KEY: &str = "inference_model";

pub async fn is_owner(state: &AppState, who: &ParticipantId) -> bool {
    matches!(
        state.store.get_setting(OWNER_KEY).await,
        Ok(Some(owner)) if owner == who.to_string()
    )
}

async fn require_owner(state: &AppState, who: &ParticipantId) -> Result<(), ApiError> {
    if is_owner(state, who).await {
        Ok(())
    } else {
        Err(ApiError(StatusCode::FORBIDDEN, "owner only".into()))
    }
}

/// Apply a provider choice to the running server. Gemini needs the API key
/// in the PROTOMOLECULE env var (secrets are never persisted in settings).
async fn apply(
    state: &Arc<AppState>,
    provider: &str,
    base: &str,
    model: &str,
) -> Result<(), ApiError> {
    match provider {
        "ollama" => {
            if base.is_empty() || model.is_empty() {
                return Err(ApiError::bad_request("ollama needs a base URL and model"));
            }
            state.inference.set(Arc::new(OllamaInference::new(
                base.to_string(),
                model.to_string(),
            )));
        }
        "gemini" => {
            let key = std::env::var("PROTOMOLECULE").map_err(|_| {
                ApiError::bad_request("no Gemini API key configured (set PROTOMOLECULE)")
            })?;
            let model = if model.is_empty() {
                "gemini-3.1-flash-lite"
            } else {
                model
            };
            state
                .inference
                .set(Arc::new(GeminiInference::new(key, model.to_string())));
        }
        other => return Err(ApiError::bad_request(format!("unknown provider: {other}"))),
    }
    Ok(())
}

/// On startup, apply any persisted inference config (overrides env defaults).
pub async fn apply_persisted(state: &Arc<AppState>) {
    let provider = state.store.get_setting(PROVIDER_KEY).await.ok().flatten();
    if let Some(provider) = provider {
        let base = state
            .store
            .get_setting(BASE_KEY)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let model = state
            .store
            .get_setting(MODEL_KEY)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        if let Err(e) = apply(state, &provider, &base, &model).await {
            tracing::warn!(err = %e.1, "persisted inference settings not applied");
        } else {
            tracing::info!(%provider, %model, "inference from persisted settings");
        }
    }
}

pub async fn get_settings(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_owner(&state, &me).await?;
    Ok(Json(serde_json::json!({
        "domain": state.domain,
        "inferenceProvider": state.store.get_setting(PROVIDER_KEY).await?.unwrap_or_default(),
        "inferenceBase": state.store.get_setting(BASE_KEY).await?.unwrap_or_default(),
        "inferenceModel": state.store.get_setting(MODEL_KEY).await?.unwrap_or_default(),
        "activeModel": state.inference.model().unwrap_or_default(),
        "geminiKeyPresent": std::env::var("PROTOMOLECULE").is_ok(),
    })))
}

#[derive(Deserialize)]
pub struct SettingsRequest {
    pub provider: String,
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub model: String,
}

pub async fn put_settings(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Json(req): Json<SettingsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_owner(&state, &me).await?;
    apply(&state, &req.provider, req.base.trim(), req.model.trim()).await?;
    state.store.put_setting(PROVIDER_KEY, &req.provider).await?;
    state.store.put_setting(BASE_KEY, req.base.trim()).await?;
    state.store.put_setting(MODEL_KEY, req.model.trim()).await?;
    tracing::info!(provider = %req.provider, model = %req.model, by = %me, "inference reconfigured");
    Ok(Json(serde_json::json!({
        "ok": true,
        "activeModel": state.inference.model().unwrap_or_default(),
    })))
}
