//! Accounts and sessions (FR-1..2).
//!
//! argon2id password hashing, httpOnly session cookies, and the
//! `CurrentUser` extractor used by both REST handlers and the WebSocket
//! upgrade (so the WS is authenticated before any protocol frame flows).

use std::sync::Arc;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{AppendHeaders, IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use protowave_core::ParticipantId;

use crate::store::{now_ms, Account};
use crate::AppState;

pub const SESSION_COOKIE: &str = "pw_session";
const SESSION_MAX_AGE_SECS: u64 = 30 * 24 * 3600;

pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        tracing::error!(%e, "store error");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "storage error".into())
    }
}

/// Extracts the authenticated participant from the session cookie.
pub struct CurrentUser(pub ParticipantId);

fn session_id_from_cookies(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| {
            let (k, v) = pair.trim().split_once('=')?;
            (k == SESSION_COOKIE).then(|| v.to_string())
        })
        .next()
}

#[axum::async_trait]
impl FromRequestParts<Arc<AppState>> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let sid = session_id_from_cookies(parts)
            .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "not logged in".into()))?;
        let participant = state
            .store
            .get_session(&sid)
            .await?
            .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "session expired".into()))?;
        Ok(CurrentUser(participant))
    }
}

fn new_session_id() -> String {
    let mut raw = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    hex::encode(raw)
}

fn session_cookie(sid: &str, max_age: u64) -> String {
    format!("{SESSION_COOKIE}={sid}; HttpOnly; Path=/; SameSite=Lax; Max-Age={max_age}")
}

#[derive(Deserialize)]
pub struct CredentialsRequest {
    /// Local part only; the server owns the domain.
    pub name: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub participant: String,
}

/// A session response: the Set-Cookie header plus the participant body.
type SessionReply = (
    AppendHeaders<[(axum::http::HeaderName, String); 1]>,
    Json<SessionResponse>,
);

async fn start_session(
    state: &AppState,
    participant: &ParticipantId,
) -> Result<SessionReply, ApiError> {
    let sid = new_session_id();
    state.store.put_session(&sid, participant).await?;
    Ok((
        AppendHeaders([(SET_COOKIE, session_cookie(&sid, SESSION_MAX_AGE_SECS))]),
        Json(SessionResponse {
            participant: participant.to_string(),
        }),
    ))
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CredentialsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.password.len() < 8 {
        return Err(ApiError::bad_request(
            "password must be at least 8 characters",
        ));
    }
    let participant = ParticipantId::new(&req.name, &state.domain)
        .map_err(|e| ApiError::bad_request(format!("invalid name: {e}")))?;

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .to_string();

    let created = state
        .store
        .create_account(&Account {
            participant: participant.to_string(),
            password_hash: hash,
            created_ms: now_ms(),
        })
        .await?;
    if !created {
        return Err(ApiError(StatusCode::CONFLICT, "name already taken".into()));
    }
    tracing::info!(%participant, "account created");
    start_session(&state, &participant).await
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CredentialsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let unauthorized = || ApiError(StatusCode::UNAUTHORIZED, "invalid credentials".into());
    let participant = ParticipantId::new(&req.name, &state.domain).map_err(|_| unauthorized())?;
    let account = state
        .store
        .get_account(&participant)
        .await?
        .ok_or_else(unauthorized)?;
    let parsed = PasswordHash::new(&account.password_hash)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed)
        .map_err(|_| unauthorized())?;
    start_session(&state, &participant).await
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    parts: axum::http::request::Parts,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(sid) = session_id_from_cookies(&parts) {
        state.store.delete_session(&sid).await?;
    }
    Ok((
        AppendHeaders([(SET_COOKIE, session_cookie("", 0))]),
        Json(serde_json::json!({ "ok": true })),
    ))
}

pub async fn me(CurrentUser(participant): CurrentUser) -> Json<SessionResponse> {
    Json(SessionResponse {
        participant: participant.to_string(),
    })
}
