use protowave_server::{app, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let state = AppState::from_env().await.expect("initialize state");
    let addr = std::env::var("PROTOWAVE_ADDR").unwrap_or_else(|_| "127.0.0.1:9898".into());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    tracing::info!(%addr, domain = %state.domain, "protowave-server listening");

    axum::serve(listener, app(state)).await.expect("server run");
}
