//! ProtoWave server (Phase 1): auth, wave lifecycle, and the collaborative
//! wave engine over the multiplexed WebSocket protocol (PRD §7, §8.1).

pub mod api;
pub mod auth;
pub mod engine;
pub mod store;
pub mod ws;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use engine::Engine;
use store::{FileStore, WaveStore};

pub struct AppState {
    pub store: Arc<dyn WaveStore>,
    pub engine: Engine,
    /// This server's federation domain (PRD §8.2).
    pub domain: String,
}

impl AppState {
    pub fn new(store: Arc<dyn WaveStore>, domain: impl Into<String>) -> Self {
        Self {
            engine: Engine::new(store.clone()),
            store,
            domain: domain.into(),
        }
    }

    pub fn from_env() -> std::io::Result<Self> {
        let domain = std::env::var("PROTOWAVE_DOMAIN").unwrap_or_else(|_| "localhost".into());
        let data_dir = std::env::var("PROTOWAVE_DATA_DIR").unwrap_or_else(|_| "data".into());
        let fsync = std::env::var("PROTOWAVE_FSYNC")
            .map(|v| v != "0")
            .unwrap_or(true);
        let store = Arc::new(FileStore::open(data_dir, fsync)?);
        Ok(Self::new(store, domain))
    }
}

pub fn app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws::ws_upgrade))
        .route("/api/register", post(auth::register))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/waves", get(api::list_waves).post(api::create_wave))
        .route("/api/waves/participants", post(api::add_participant))
        .with_state(state)
}
