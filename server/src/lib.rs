//! ProtoWave server (Phase 2): auth, wave lifecycle, collaborative engine,
//! attachments, playback, and full-text search over the multiplexed
//! WebSocket + REST protocol (PRD §7, §8.1).

pub mod api;
pub mod attachments;
pub mod auth;
pub mod cas;
pub mod engine;
pub mod federation;
pub mod search;
pub mod shares;
pub mod store;
pub mod store_pg;
pub mod translate;
pub mod ws;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

use cas::Cas;
use engine::Engine;
use federation::{Federation, FederationConfig};
use search::{SearchIndex, TantivyIndex};
use store::{FileStore, WaveStore};
use store_pg::PgStore;

pub struct AppState {
    pub store: Arc<dyn WaveStore>,
    pub engine: Engine,
    pub cas: Cas,
    pub search: Arc<dyn SearchIndex>,
    pub federation: Federation,
    pub translation: translate::TranslationHub,
    /// This server's federation domain (PRD §8.2).
    pub domain: String,
}

impl AppState {
    /// `data_dir` hosts the CAS blobs, search index and server signing key
    /// regardless of which WaveStore backend is in use.
    pub fn build(
        store: Arc<dyn WaveStore>,
        domain: impl Into<String>,
        data_dir: &Path,
        fsync: bool,
        fed_config: FederationConfig,
    ) -> std::io::Result<Arc<Self>> {
        let call_cap = std::env::var("PROTOWAVE_TRANSLATE_CAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5000);
        let state = Arc::new(Self {
            engine: Engine::new(store.clone()),
            store,
            cas: Cas::open(data_dir.join("blobs"), fsync)?,
            search: Arc::new(TantivyIndex::open(&data_dir.join("search"))?),
            federation: Federation::new(fed_config, data_dir)?,
            translation: translate::TranslationHub::new(call_cap),
            domain: domain.into(),
        });
        // Gemini Flash-Lite-class reference provider (FR-46); swap via env.
        if let Ok(key) = std::env::var("PROTOMOLECULE") {
            let model = std::env::var("PROTOWAVE_TRANSLATE_MODEL")
                .unwrap_or_else(|_| "gemini-3.1-flash-lite".into());
            tracing::info!(%model, "translation provider configured");
            state
                .translation
                .set_translator(Arc::new(translate::GeminiTranslator::new(key, model)));
        }
        spawn_search_indexer(state.clone());
        translate::spawn_translation_worker(state.clone());
        Ok(state)
    }

    pub async fn from_env() -> std::io::Result<Arc<Self>> {
        let domain = std::env::var("PROTOWAVE_DOMAIN").unwrap_or_else(|_| "localhost".into());
        let data_dir = std::env::var("PROTOWAVE_DATA_DIR").unwrap_or_else(|_| "data".into());
        let fsync = std::env::var("PROTOWAVE_FSYNC")
            .map(|v| v != "0")
            .unwrap_or(true);
        let store: Arc<dyn WaveStore> = match std::env::var("PROTOWAVE_PG") {
            Ok(url) => {
                tracing::info!("using PostgreSQL WaveStore");
                Arc::new(PgStore::connect(&url).await?)
            }
            Err(_) => Arc::new(FileStore::open(&data_dir, fsync)?),
        };
        // PROTOWAVE_PEERS: "domainA=http://host:port,domainB=http://..."
        let peers = std::env::var("PROTOWAVE_PEERS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|pair| {
                let (d, url) = pair.trim().split_once('=')?;
                Some((d.to_string(), url.to_string()))
            })
            .collect();
        let fed_config = FederationConfig {
            public_url: std::env::var("PROTOWAVE_PUBLIC_URL").unwrap_or_default(),
            peers,
        };
        Self::build(store, domain, Path::new(&data_dir), fsync, fed_config)
    }
}

/// Incremental search indexing (FR-29): consume the engine's change stream,
/// coalesce bursts, re-extract per-wave text.
fn spawn_search_indexer(state: Arc<AppState>) {
    let mut rx = state.engine.change_stream();
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Coalesce whatever else arrived in the meantime.
            let mut batch = HashSet::new();
            batch.insert(first);
            while let Ok(more) = rx.try_recv() {
                batch.insert(more);
            }
            for name in batch {
                let wave_key = name.wave_id.to_string();
                let title = match state.store.get_wave(&wave_key).await {
                    Ok(Some(meta)) => meta.title,
                    _ => continue,
                };
                if let Ok(live) = state.engine.open_wavelet(&name).await {
                    let body = live.extract_text();
                    if let Err(e) = state.search.upsert(&wave_key, &title, &body) {
                        tracing::warn!(%e, wave = %wave_key, "search upsert failed");
                    }
                }
            }
            // Light rate limit; indexing lags edits by at most this.
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}

pub fn app(state: Arc<AppState>) -> Router {
    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws::ws_upgrade))
        .route("/api/register", post(auth::register))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/waves", get(api::list_waves).post(api::create_wave))
        .route("/api/waves/participants", post(api::add_participant))
        .route("/api/waves/read", post(api::mark_read))
        .route("/api/waves/translation", post(api::set_translation))
        .route("/api/history", get(api::history))
        .route("/api/search", get(api::search))
        .route(
            "/api/attachments",
            get(attachments::list).post(attachments::upload),
        )
        .route("/api/attachments/:hash", get(attachments::download))
        .route("/api/shares", get(shares::list).post(shares::upload))
        .route("/api/shares/:hash", get(shares::manifest))
        .route("/api/shares/:hash/file", get(shares::download))
        .route("/api/shares/:hash/mirror", post(shares::mirror))
        .route("/.well-known/protowave", get(federation::well_known))
        .route(federation::PUSH_PATH, post(federation::handle_push))
        .route(federation::SYNC_PATH, post(federation::handle_sync))
        .route(federation::ANNOUNCE_PATH, post(federation::handle_announce))
        .route(
            shares::SHARE_ANNOUNCE_PATH,
            post(shares::handle_share_announce),
        )
        .route(shares::BLOB_PATH, post(shares::handle_blob))
        // Folder uploads are large; per-handler checks enforce tighter caps.
        .layer(axum::extract::DefaultBodyLimit::max(
            shares::MAX_UPLOAD_BYTES,
        ))
        .with_state(state);

    // Single-binary deploy (G11): serve the built SPA when present, with
    // index.html fallback for client-side routes.
    let dist = std::env::var("PROTOWAVE_WEB_DIST").unwrap_or_else(|_| "web/dist".into());
    if Path::new(&dist).join("index.html").exists() {
        let spa = ServeDir::new(&dist)
            .not_found_service(ServeFile::new(Path::new(&dist).join("index.html")));
        router.fallback_service(spa)
    } else {
        router
    }
}
