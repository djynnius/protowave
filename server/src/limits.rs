//! Anti-abuse rate limiting (FR-63): a fixed-window counter per
//! (client key, action). In-memory — resets on restart, which is fine for
//! its purpose (blunting brute force and runaway automation, not billing).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::Json;

use crate::auth::{ApiError, CurrentUser};
use crate::AppState;

/// (client key, action) → (window start, events in window).
type Windows = HashMap<(String, &'static str), (Instant, u32)>;

pub struct RateLimiter {
    windows: Mutex<Windows>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
        }
    }
}

impl RateLimiter {
    /// Allow up to `max` events per `window` for (key, action).
    pub fn check(
        &self,
        key: &str,
        action: &'static str,
        max: u32,
        window: Duration,
    ) -> Result<(), ApiError> {
        let mut windows = self.windows.lock().unwrap();
        // Opportunistic cleanup so the map can't grow without bound.
        if windows.len() > 10_000 {
            let now = Instant::now();
            windows.retain(|_, (start, _)| now.duration_since(*start) < window);
        }
        let now = Instant::now();
        let entry = windows.entry((key.to_string(), action)).or_insert((now, 0));
        if now.duration_since(entry.0) >= window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        if entry.1 > max {
            tracing::warn!(%key, action, "rate limit exceeded");
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "slow down — rate limit exceeded".into(),
            ));
        }
        Ok(())
    }
}

/// Client key for unauthenticated endpoints: the peer IP.
pub fn client_key(addr: &SocketAddr) -> String {
    addr.ip().to_string()
}

// ---------------------------------------------------------------------------
// Admin (FR-61 basics)
// ---------------------------------------------------------------------------

/// The admin is the local account named by PROTOWAVE_ADMIN (local part).
fn require_admin(state: &AppState, user: &protowave_core::ParticipantId) -> Result<(), ApiError> {
    let admin = std::env::var("PROTOWAVE_ADMIN").unwrap_or_default();
    if !admin.is_empty() && user.local() == admin && user.domain() == state.domain {
        Ok(())
    } else {
        Err(ApiError(StatusCode::FORBIDDEN, "admin only".into()))
    }
}

pub async fn admin_stats(
    State(state): State<std::sync::Arc<AppState>>,
    CurrentUser(me): CurrentUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&state, &me)?;
    // Cheap portable stats via existing store queries.
    let waves = state.store.list_waves_for(&me).await?.len();
    Ok(Json(serde_json::json!({
        "domain": state.domain,
        "adminWaves": waves,
        "version": env!("CARGO_PKG_VERSION"),
        "protocolVersion": crate::federation::PROTOCOL_VERSION,
    })))
}

/// Extractor-style guard usable inside handlers.
pub fn limit_ip(
    state: &AppState,
    conn: &ConnectInfo<SocketAddr>,
    action: &'static str,
    max: u32,
    window_secs: u64,
) -> Result<(), ApiError> {
    state.limits.check(
        &client_key(&conn.0),
        action,
        max,
        Duration::from_secs(window_secs),
    )
}
