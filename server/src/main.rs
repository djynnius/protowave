use std::sync::Arc;

use protowave_server::{app, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let addr = std::env::var("PROTOWAVE_ADDR").unwrap_or_else(|_| "127.0.0.1:9898".into());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    tracing::info!(%addr, "protowave-server listening");

    axum::serve(listener, app(Arc::new(AppState::from_env())))
        .await
        .expect("server run");
}
