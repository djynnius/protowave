//! Inference pool management (PRD §12.1, FI-1). Any signed-in user can
//! register an Ollama-compatible endpoint they host; the auto-router in
//! `agent::select_route` then draws on the pool. No secrets are stored —
//! entries are just a base URL + model tag + a serving scope.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::agent::{InferenceProvider, OllamaInference};
use crate::auth::{ApiError, CurrentUser};
use crate::store::{now_ms, UserModel};
use crate::AppState;

/// Hard cap per user — the pool is cooperative, not a free-for-all.
const MAX_PER_USER: usize = 10;
const SCOPES: [&str; 3] = ["private", "wave", "federation"];

#[derive(Serialize)]
pub struct ModelView {
    pub id: String,
    pub owner: String,
    #[serde(rename = "ownerName")]
    pub owner_name: String,
    pub label: String,
    pub base: String,
    pub model: String,
    pub scope: String,
    pub enabled: bool,
    /// True for the caller's own entries (client shows edit/delete controls).
    pub mine: bool,
}

fn owner_local(owner: &str) -> String {
    owner.split('@').next().unwrap_or(owner).to_string()
}

fn view(m: UserModel, me: &str) -> ModelView {
    ModelView {
        owner_name: owner_local(&m.owner),
        mine: m.owner == me,
        id: m.id,
        owner: m.owner,
        label: m.label,
        base: m.base,
        model: m.model,
        scope: m.scope,
        enabled: m.enabled,
    }
}

/// GET /api/models — the caller's own models plus the visible pool (models
/// others have shared to the wave/federation scope). Bases are shown; there
/// are no secrets to redact.
pub async fn list_models(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    let me = me.to_string();
    let all = state.store.list_models().await?;
    let mut mine = Vec::new();
    let mut pool = Vec::new();
    for m in all {
        if m.owner == me {
            mine.push(view(m, &me));
        } else if m.enabled && (m.scope == "wave" || m.scope == "federation") {
            pool.push(view(m, &me));
        }
    }
    Ok(Json(serde_json::json!({ "mine": mine, "pool": pool })))
}

#[derive(Deserialize)]
pub struct ModelRequest {
    /// Present when editing an existing entry; a new id is minted otherwise.
    #[serde(default)]
    pub id: String,
    pub label: String,
    pub base: String,
    pub model: String,
    pub scope: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn validate(req: &ModelRequest) -> Result<(), ApiError> {
    let base = req.base.trim();
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(ApiError::bad_request("base must be an http(s) URL"));
    }
    if req.model.trim().is_empty() || req.model.len() > 200 {
        return Err(ApiError::bad_request("model tag required"));
    }
    if req.label.len() > 100 {
        return Err(ApiError::bad_request("label too long"));
    }
    if !SCOPES.contains(&req.scope.as_str()) {
        return Err(ApiError::bad_request(
            "scope must be private, wave or federation",
        ));
    }
    Ok(())
}

/// POST /api/models — add a new model or update one the caller owns.
pub async fn put_model(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Json(req): Json<ModelRequest>,
) -> Result<Json<ModelView>, ApiError> {
    state.limits.check(
        &me.to_string(),
        "model-write",
        60,
        Duration::from_secs(3600),
    )?;
    validate(&req)?;
    let me = me.to_string();

    let (id, created_ms) = if req.id.is_empty() {
        // New entry — enforce the per-user cap.
        let count = state
            .store
            .list_models_for(&me)
            .await?
            .into_iter()
            .filter(|m| m.id != req.id)
            .count();
        if count >= MAX_PER_USER {
            return Err(ApiError::bad_request(format!(
                "at most {MAX_PER_USER} models per user"
            )));
        }
        let mut raw = [0u8; 6];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        (format!("m+{}", hex::encode(raw)), now_ms())
    } else {
        // Editing — must exist and belong to the caller.
        let existing = state
            .store
            .get_model(&req.id)
            .await?
            .ok_or_else(|| ApiError::bad_request("no such model"))?;
        if existing.owner != me {
            return Err(ApiError::forbidden("not your model"));
        }
        (existing.id, existing.created_ms)
    };

    let model = UserModel {
        id,
        owner: me.clone(),
        label: req.label.trim().to_string(),
        base: req.base.trim().trim_end_matches('/').to_string(),
        model: req.model.trim().to_string(),
        scope: req.scope,
        enabled: req.enabled,
        created_ms,
    };
    state.store.put_model(&model).await?;
    tracing::info!(id = %model.id, owner = %me, model = %model.model, scope = %model.scope, "pool model saved");
    Ok(Json(view(model, &me)))
}

/// DELETE /api/models/:id — owner only.
pub async fn delete_model(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let me = me.to_string();
    match state.store.get_model(&id).await? {
        Some(m) if m.owner == me => {
            state.store.delete_model(&id).await?;
            Ok(Json(serde_json::json!({ "ok": true })))
        }
        Some(_) => Err(ApiError::forbidden("not your model")),
        None => Ok(Json(serde_json::json!({ "ok": true }))),
    }
}

#[derive(Deserialize)]
pub struct TestRequest {
    pub base: String,
    pub model: String,
}

/// POST /api/models/test — probe an endpoint before saving it, so the user
/// gets immediate "reachable / not reachable" feedback. Bounded so a dead
/// host can't hang the request.
pub async fn test_model(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Json(req): Json<TestRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .limits
        .check(&me.to_string(), "model-test", 60, Duration::from_secs(3600))?;
    let provider = OllamaInference::new(req.base.trim().to_string(), req.model.trim().to_string());
    let probe = tokio::time::timeout(
        Duration::from_secs(20),
        provider.infer("Reply with the single word: ok", ""),
    )
    .await;
    match probe {
        Ok(Ok(_)) => Ok(Json(
            serde_json::json!({ "ok": true, "model": provider.model() }),
        )),
        Ok(Err(e)) => Ok(Json(
            serde_json::json!({ "ok": false, "error": e.to_string() }),
        )),
        Err(_) => Ok(Json(
            serde_json::json!({ "ok": false, "error": "timed out" }),
        )),
    }
}
