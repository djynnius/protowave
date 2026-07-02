//! Phase 0 exit criterion: a client connects, authenticates on the control
//! channel, and the server echoes envelopes on the echo channel.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio_tungstenite::tungstenite::Message;

use protowave_proto::v1::{AuthRequest, AuthResponse, Channel, Envelope};
use protowave_server::{app, AppState};

async fn spawn_server() -> String {
    let state = Arc::new(AppState {
        dev_token: "dev".into(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    format!("ws://{addr}/ws")
}

async fn recv_envelope(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> Envelope {
    loop {
        match ws.next().await.expect("stream open").expect("frame") {
            Message::Binary(bytes) => return Envelope::decode_frame(&bytes).unwrap(),
            _ => continue,
        }
    }
}

#[tokio::test]
async fn auth_then_echo() {
    let url = spawn_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Authenticate.
    let auth = Envelope::control(&AuthRequest {
        participant: "ada@example.org".into(),
        token: "dev".into(),
    });
    ws.send(Message::Binary(auth.encode_frame())).await.unwrap();
    let reply = recv_envelope(&mut ws).await;
    assert_eq!(reply.channel, Channel::Control as i32);
    let auth_reply = AuthResponse::decode(reply.payload.as_slice()).unwrap();
    assert!(auth_reply.ok, "auth failed: {}", auth_reply.error);
    assert!(!auth_reply.session_id.is_empty());

    // Echo.
    let echo = Envelope::new(Channel::Echo, b"hello wave".to_vec());
    ws.send(Message::Binary(echo.encode_frame())).await.unwrap();
    let reply = recv_envelope(&mut ws).await;
    assert_eq!(reply.channel, Channel::Echo as i32);
    assert_eq!(reply.payload, b"hello wave");
}

#[tokio::test]
async fn echo_requires_auth() {
    let url = spawn_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    let echo = Envelope::new(Channel::Echo, b"sneaky".to_vec());
    ws.send(Message::Binary(echo.encode_frame())).await.unwrap();
    let reply = recv_envelope(&mut ws).await;
    assert_eq!(reply.channel, Channel::Control as i32);
    let auth_reply = AuthResponse::decode(reply.payload.as_slice()).unwrap();
    assert!(!auth_reply.ok);
    assert_eq!(auth_reply.error, "unauthenticated");
}

#[tokio::test]
async fn bad_token_rejected() {
    let url = spawn_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    let auth = Envelope::control(&AuthRequest {
        participant: "ada@example.org".into(),
        token: "wrong".into(),
    });
    ws.send(Message::Binary(auth.encode_frame())).await.unwrap();
    let reply = recv_envelope(&mut ws).await;
    let auth_reply = AuthResponse::decode(reply.payload.as_slice()).unwrap();
    assert!(!auth_reply.ok);
}
