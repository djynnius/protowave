//! ProtoWave server library (Phase 0).
//!
//! Exposes the axum `Router` so integration tests can bind it to an
//! ephemeral port. Scope is the PRD Phase 0 exit criterion: a client
//! connects over WebSocket, authenticates on the control channel, and the
//! server echoes envelopes on the echo channel.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use prost::Message as _;
use rand::RngCore;

use protowave_core::ParticipantId;
use protowave_proto::v1::{AuthRequest, AuthResponse, Channel, Envelope};

pub struct AppState {
    /// Phase 0 stub credential (FR-1..2 replace this in Phase 1).
    pub dev_token: String,
}

impl AppState {
    pub fn from_env() -> Self {
        Self {
            dev_token: std::env::var("PROTOWAVE_DEV_TOKEN").unwrap_or_else(|_| "dev".into()),
        }
    }
}

pub fn app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws_upgrade))
        .with_state(state)
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| session(socket, state))
}

/// One connected client session: unauthenticated until a valid AuthRequest
/// arrives on the control channel, then echo service.
async fn session(mut socket: WebSocket, state: Arc<AppState>) {
    let mut session_id: Option<String> = None;

    while let Some(Ok(msg)) = socket.recv().await {
        let frame = match msg {
            Message::Binary(bytes) => bytes,
            Message::Close(_) => break,
            // Ping/pong handled by axum; text frames are not part of the protocol.
            _ => continue,
        };

        let envelope = match Envelope::decode_frame(&frame) {
            Ok(env) => env,
            Err(err) => {
                tracing::debug!(%err, "undecodable frame, closing");
                break;
            }
        };

        let reply = match (Channel::try_from(envelope.channel), &session_id) {
            (Ok(Channel::Control), _) => {
                let (response, sid) = authenticate(&envelope.payload, &state);
                session_id = sid;
                Some(Envelope::control(&response))
            }
            (Ok(Channel::Echo), Some(_)) => Some(envelope),
            (Ok(Channel::Echo), None) => Some(Envelope::control(&AuthResponse {
                ok: false,
                session_id: String::new(),
                error: "unauthenticated".into(),
            })),
            _ => {
                tracing::debug!(channel = envelope.channel, "unsupported channel in Phase 0");
                None
            }
        };

        if let Some(reply) = reply {
            if socket
                .send(Message::Binary(reply.encode_frame()))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

fn authenticate(payload: &[u8], state: &AppState) -> (AuthResponse, Option<String>) {
    let reject = |error: &str| AuthResponse {
        ok: false,
        session_id: String::new(),
        error: error.into(),
    };

    let request = match AuthRequest::decode(payload) {
        Ok(req) => req,
        Err(_) => return (reject("malformed auth request"), None),
    };
    let participant = match request.participant.parse::<ParticipantId>() {
        Ok(p) => p,
        Err(err) => return (reject(&format!("invalid participant: {err}")), None),
    };
    if request.token != state.dev_token {
        return (reject("invalid token"), None);
    }

    let mut raw = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let sid = hex::encode(raw);
    tracing::info!(%participant, session = %sid, "session authenticated");
    (
        AuthResponse {
            ok: true,
            session_id: sid.clone(),
            error: String::new(),
        },
        Some(sid),
    )
}
