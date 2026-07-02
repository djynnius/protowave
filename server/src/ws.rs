//! WebSocket sessions (FR-7, FR-12..13, FR-22..23).
//!
//! The socket is authenticated at upgrade time via the session cookie.
//! Each session multiplexes wavelet subscriptions: Subscribe performs the
//! state-vector/diff exchange (NFR-21 reconnect convergence), then a
//! forwarder task per wavelet streams broadcast events into the session's
//! outbound queue.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use prost::Message as _;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use protowave_core::{ParticipantId, WaveletName};
use protowave_proto::v1 as pb;
use protowave_proto::v1::{control_message, sync_message};

use crate::auth::CurrentUser;
use crate::engine::{EngineError, EventKind, LiveWavelet};
use crate::AppState;

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    CurrentUser(user): CurrentUser,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| session(socket, state, user))
}

struct Subscription {
    live: Arc<LiveWavelet>,
    forwarder: JoinHandle<()>,
}

fn control_frame(kind: control_message::Kind) -> Vec<u8> {
    pb::Envelope::new(
        pb::Channel::Control,
        pb::ControlMessage { kind: Some(kind) }.encode_to_vec(),
    )
    .encode_frame()
}

fn sync_frame(wavelet: &str, kind: sync_message::Kind) -> Vec<u8> {
    pb::Envelope::new(
        pb::Channel::Sync,
        pb::SyncMessage {
            wavelet: wavelet.to_string(),
            kind: Some(kind),
        }
        .encode_to_vec(),
    )
    .encode_frame()
}

fn awareness_frame(wavelet: &str, payload: Vec<u8>) -> Vec<u8> {
    pb::Envelope::new(
        pb::Channel::Awareness,
        pb::AwarenessMessage {
            wavelet: wavelet.to_string(),
            payload,
        }
        .encode_to_vec(),
    )
    .encode_frame()
}

fn error_frame(wavelet: &str, code: &str, message: &str) -> Vec<u8> {
    control_frame(control_message::Kind::Error(pb::ControlError {
        wavelet: wavelet.to_string(),
        code: code.to_string(),
        message: message.to_string(),
    }))
}

async fn session(mut socket: WebSocket, state: Arc<AppState>, user: ParticipantId) {
    let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(256);
    let mut subs: HashMap<String, Subscription> = HashMap::new();
    tracing::info!(%user, conn = conn_id, "ws session open");

    loop {
        tokio::select! {
            inbound = socket.recv() => {
                let frame = match inbound {
                    Some(Ok(Message::Binary(bytes))) => bytes,
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(_)) => break,
                };
                if let Some(reply) =
                    handle_frame(&state, &user, conn_id, &frame, &mut subs, &out_tx).await
                {
                    if socket.send(Message::Binary(reply)).await.is_err() {
                        break;
                    }
                }
            }
            outbound = out_rx.recv() => {
                match outbound {
                    Some(frame) => {
                        if socket.send(Message::Binary(frame)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    for (_, sub) in subs {
        sub.live.drop_awareness(conn_id);
        sub.forwarder.abort();
    }
    tracing::info!(%user, conn = conn_id, "ws session closed");
}

/// Handle one inbound frame; the immediate reply (if any) is returned, while
/// subscription streams flow through `out_tx`.
async fn handle_frame(
    state: &Arc<AppState>,
    user: &ParticipantId,
    conn_id: u64,
    frame: &[u8],
    subs: &mut HashMap<String, Subscription>,
    out_tx: &mpsc::Sender<Vec<u8>>,
) -> Option<Vec<u8>> {
    let envelope = pb::Envelope::decode_frame(frame).ok()?;
    match pb::Channel::try_from(envelope.channel) {
        Ok(pb::Channel::Control) => {
            let msg = pb::ControlMessage::decode(envelope.payload.as_slice()).ok()?;
            match msg.kind? {
                control_message::Kind::Subscribe(sub) => {
                    Some(subscribe(state, user, conn_id, sub, subs, out_tx).await)
                }
                control_message::Kind::Unsubscribe(unsub) => {
                    if let Some(sub) = subs.remove(&unsub.wavelet) {
                        sub.live.drop_awareness(conn_id);
                        sub.forwarder.abort();
                    }
                    None
                }
                _ => None,
            }
        }
        Ok(pb::Channel::Sync) => {
            let msg = pb::SyncMessage::decode(envelope.payload.as_slice()).ok()?;
            let sub = match subs.get(&msg.wavelet) {
                Some(s) => s,
                None => {
                    return Some(error_frame(
                        &msg.wavelet,
                        "not-subscribed",
                        "subscribe first",
                    ))
                }
            };
            if let Some(sync_message::Kind::Update(update)) = msg.kind {
                let bytes = update.update.clone();
                match state
                    .engine
                    .apply_update(&sub.live, update.update, conn_id)
                    .await
                {
                    Ok(()) => {
                        // Locally-originated: fan out to federation peers
                        // (FR-48). Remote-applied updates never take this
                        // path, so there is no relay loop.
                        crate::federation::spawn_push_update(
                            state.clone(),
                            sub.live.name.clone(),
                            bytes,
                        );
                        None
                    }
                    Err(EngineError::BadPayload(e)) => {
                        Some(error_frame(&msg.wavelet, "bad-update", &e))
                    }
                    Err(e) => {
                        tracing::error!(?e, wavelet = %msg.wavelet, "apply_update failed");
                        Some(error_frame(&msg.wavelet, "internal", "update failed"))
                    }
                }
            } else {
                None
            }
        }
        Ok(pb::Channel::Awareness) => {
            let msg = pb::AwarenessMessage::decode(envelope.payload.as_slice()).ok()?;
            if let Some(sub) = subs.get(&msg.wavelet) {
                state
                    .engine
                    .relay_awareness(&sub.live, msg.payload, conn_id);
            }
            None
        }
        // Phase 0 smoke channel.
        Ok(pb::Channel::Echo) => Some(frame.to_vec()),
        _ => None,
    }
}

async fn subscribe(
    state: &Arc<AppState>,
    user: &ParticipantId,
    conn_id: u64,
    req: pb::Subscribe,
    subs: &mut HashMap<String, Subscription>,
    out_tx: &mpsc::Sender<Vec<u8>>,
) -> Vec<u8> {
    let name: WaveletName = match req.wavelet.parse() {
        Ok(n) => n,
        Err(e) => return error_frame(&req.wavelet, "bad-request", &e.to_string()),
    };

    // ACL: only wave participants may subscribe (FR-6).
    let wave_key = name.wave_id.to_string();
    match state.store.get_wave(&wave_key).await {
        Ok(Some(meta)) if meta.participants.contains(&user.to_string()) => {}
        Ok(Some(_)) => return error_frame(&req.wavelet, "not-participant", "not on this wave"),
        Ok(None) => return error_frame(&req.wavelet, "not-found", "no such wave"),
        Err(e) => {
            tracing::error!(%e, "store error during subscribe");
            return error_frame(&req.wavelet, "internal", "storage error");
        }
    }

    let live = match state.engine.open_wavelet(&name).await {
        Ok(live) => live,
        Err(e) => {
            tracing::error!(?e, wavelet = %req.wavelet, "open_wavelet failed");
            return error_frame(&req.wavelet, "internal", "open failed");
        }
    };

    // Remote-homed wave: freshen our replica via anti-entropy (FR-50).
    if name.wave_id.domain() != state.domain {
        crate::federation::spawn_sync_pull(state.clone(), name.clone());
    }

    let (server_sv, diff) = match live.sync_state(&req.state_vector) {
        Ok(pair) => pair,
        Err(_) => return error_frame(&req.wavelet, "bad-request", "invalid state vector"),
    };

    // Forwarder: wavelet broadcast → this session's outbound queue.
    let mut rx = live.broadcast.subscribe();
    let wavelet_key = req.wavelet.clone();
    let tx = out_tx.clone();
    let forwarder = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if event.from == conn_id {
                        continue;
                    }
                    let frame = match event.kind {
                        EventKind::Update(bytes) => sync_frame(
                            &wavelet_key,
                            sync_message::Kind::Update(pb::Update {
                                update: bytes.as_ref().clone(),
                            }),
                        ),
                        EventKind::Awareness(bytes) => {
                            awareness_frame(&wavelet_key, bytes.as_ref().clone())
                        }
                    };
                    if tx.send(frame).await.is_err() {
                        break;
                    }
                }
                // Lagged: skip missed fanout; the doc converges via the next
                // subscribe sync. Closed: wavelet evicted.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Replay cached awareness of existing subscribers to the newcomer.
    for payload in live.cached_awareness() {
        let _ = out_tx.try_send(awareness_frame(&req.wavelet, payload.as_ref().clone()));
    }

    subs.insert(req.wavelet.clone(), Subscription { live, forwarder });

    // Subscribed ack, then the sync state on the sync channel.
    let _ = out_tx.try_send(sync_frame(
        &req.wavelet,
        sync_message::Kind::SyncState(pb::SyncState {
            state_vector: server_sv,
            diff,
        }),
    ));
    control_frame(control_message::Kind::Subscribed(pb::Subscribed {
        wavelet: req.wavelet,
    }))
}
