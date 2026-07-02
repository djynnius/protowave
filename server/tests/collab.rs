//! Phase 1 exit criterion (PRD §12): two users co-edit a threaded wave —
//! auth via REST + session cookie, WS subscribe with state-vector sync,
//! concurrent edits converge, offline reconnect converges (NFR-21),
//! awareness relays, and non-participants are rejected.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::util::ServiceExt;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, Text, Transact, Update};

use protowave_proto::v1 as pb;
use protowave_proto::v1::{control_message, sync_message};
use protowave_server::store::FileStore;
use protowave_server::{app, AppState};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct TestServer {
    router: axum::Router,
    ws_url: String,
    _dir: tempfile::TempDir,
}

async fn spawn() -> TestServer {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
    let state = Arc::new(AppState::new(store, "localhost"));
    let router = app(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_router = router.clone();
    tokio::spawn(async move {
        axum::serve(listener, serve_router).await.unwrap();
    });
    TestServer {
        router,
        ws_url: format!("ws://{addr}/ws"),
        _dir: dir,
    }
}

impl TestServer {
    /// REST call via tower::oneshot; returns (status, session-cookie, body).
    async fn post(
        &self,
        path: &str,
        cookie: Option<&str>,
        body: serde_json::Value,
    ) -> (StatusCode, Option<String>, serde_json::Value) {
        let mut req = Request::post(path).header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        let res = self
            .router
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let set_cookie = res
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, set_cookie, json)
    }

    async fn register(&self, name: &str) -> String {
        let (status, cookie, _) = self
            .post(
                "/api/register",
                None,
                serde_json::json!({ "name": name, "password": "correct horse battery" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "register {name}");
        cookie.expect("session cookie")
    }

    async fn connect_ws(&self, cookie: &str) -> Ws {
        let mut req = self.ws_url.clone().into_client_request().unwrap();
        req.headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }
}

// ---- protocol helpers ----

fn subscribe_frame(wavelet: &str, sv: Vec<u8>) -> Message {
    let msg = pb::ControlMessage {
        kind: Some(control_message::Kind::Subscribe(pb::Subscribe {
            wavelet: wavelet.into(),
            state_vector: sv,
        })),
    };
    Message::Binary(pb::Envelope::control(&msg).encode_frame())
}

fn update_frame(wavelet: &str, update: Vec<u8>) -> Message {
    let msg = pb::SyncMessage {
        wavelet: wavelet.into(),
        kind: Some(sync_message::Kind::Update(pb::Update { update })),
    };
    Message::Binary(pb::Envelope::new(pb::Channel::Sync, msg.encode_to_vec()).encode_frame())
}

fn awareness_frame(wavelet: &str, payload: Vec<u8>) -> Message {
    let msg = pb::AwarenessMessage {
        wavelet: wavelet.into(),
        payload,
    };
    Message::Binary(pb::Envelope::new(pb::Channel::Awareness, msg.encode_to_vec()).encode_frame())
}

enum Inbound {
    Subscribed(String),
    SyncState {
        state_vector: Vec<u8>,
        diff: Vec<u8>,
    },
    Update(Vec<u8>),
    Awareness(Vec<u8>),
    Error(String),
}

async fn recv(ws: &mut Ws) -> Inbound {
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("recv timeout")
            .expect("stream open")
            .expect("frame");
        let bytes = match frame {
            Message::Binary(b) => b,
            _ => continue,
        };
        let env = pb::Envelope::decode_frame(&bytes).unwrap();
        match pb::Channel::try_from(env.channel).unwrap() {
            pb::Channel::Control => {
                let msg = pb::ControlMessage::decode(env.payload.as_slice()).unwrap();
                match msg.kind.unwrap() {
                    control_message::Kind::Subscribed(s) => return Inbound::Subscribed(s.wavelet),
                    control_message::Kind::Error(e) => return Inbound::Error(e.code),
                    _ => continue,
                }
            }
            pb::Channel::Sync => {
                let msg = pb::SyncMessage::decode(env.payload.as_slice()).unwrap();
                match msg.kind.unwrap() {
                    sync_message::Kind::SyncState(s) => {
                        return Inbound::SyncState {
                            state_vector: s.state_vector,
                            diff: s.diff,
                        }
                    }
                    sync_message::Kind::Update(u) => return Inbound::Update(u.update),
                }
            }
            pb::Channel::Awareness => {
                let msg = pb::AwarenessMessage::decode(env.payload.as_slice()).unwrap();
                return Inbound::Awareness(msg.payload);
            }
            _ => continue,
        }
    }
}

/// Subscribe and drain the Subscribed + SyncState pair, applying the diff.
async fn open_wavelet(ws: &mut Ws, wavelet: &str, doc: &Doc) {
    let sv = doc.transact().state_vector().encode_v1();
    ws.send(subscribe_frame(wavelet, sv)).await.unwrap();
    let mut subscribed = false;
    let mut synced = false;
    while !(subscribed && synced) {
        match recv(ws).await {
            Inbound::Subscribed(_) => subscribed = true,
            Inbound::SyncState { diff, .. } => {
                if !diff.is_empty() {
                    doc.transact_mut()
                        .apply_update(Update::decode_v1(&diff).unwrap());
                }
                synced = true;
            }
            Inbound::Error(code) => panic!("subscribe failed: {code}"),
            _ => {}
        }
    }
}

fn edit(doc: &Doc, text: &str) -> Vec<u8> {
    let t = doc.get_or_insert_text("body");
    let before = doc.transact().state_vector();
    {
        let mut txn = doc.transact_mut();
        let len = t.get_string(&txn).len() as u32;
        t.insert(&mut txn, len, text);
    }
    doc.transact().encode_diff_v1(&before)
}

fn body_text(doc: &Doc) -> String {
    let t = doc.get_or_insert_text("body");
    let txn = doc.transact();
    t.get_string(&txn)
}

async fn create_wave(server: &TestServer, cookie: &str, title: &str) -> (String, String) {
    let (status, _, body) = server
        .post(
            "/api/waves",
            Some(cookie),
            serde_json::json!({ "title": title }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    (
        body["wave"].as_str().unwrap().to_string(),
        body["rootWavelet"].as_str().unwrap().to_string(),
    )
}

// ---- tests ----

#[tokio::test]
async fn two_users_coedit_and_converge() {
    let server = spawn().await;
    let ada_cookie = server.register("ada").await;
    let bob_cookie = server.register("bob").await;

    let (wave, root) = create_wave(&server, &ada_cookie, "Phase 1 exit").await;
    let (status, _, _) = server
        .post(
            "/api/waves/participants",
            Some(&ada_cookie),
            serde_json::json!({ "wave": wave, "participant": "bob@localhost" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let mut ada_ws = server.connect_ws(&ada_cookie).await;
    let mut bob_ws = server.connect_ws(&bob_cookie).await;
    let ada_doc = Doc::new();
    let bob_doc = Doc::new();
    open_wavelet(&mut ada_ws, &root, &ada_doc).await;
    open_wavelet(&mut bob_ws, &root, &bob_doc).await;

    // Concurrent edits in both directions.
    ada_ws
        .send(update_frame(&root, edit(&ada_doc, "hello ")))
        .await
        .unwrap();
    bob_ws
        .send(update_frame(&root, edit(&bob_doc, "world")))
        .await
        .unwrap();

    // Each applies the other's update.
    for (ws, doc) in [(&mut ada_ws, &ada_doc), (&mut bob_ws, &bob_doc)] {
        match recv(ws).await {
            Inbound::Update(u) => doc
                .transact_mut()
                .apply_update(Update::decode_v1(&u).unwrap()),
            _ => panic!("expected update"),
        }
    }

    assert_eq!(body_text(&ada_doc), body_text(&bob_doc));
    assert!(body_text(&ada_doc).contains("hello"));
    assert!(body_text(&ada_doc).contains("world"));
}

#[tokio::test]
async fn offline_reconnect_converges() {
    let server = spawn().await;
    let ada_cookie = server.register("ada").await;
    let (_wave, root) = create_wave(&server, &ada_cookie, "reconnect").await;

    // First session: write, then go offline.
    let doc = Doc::new();
    {
        let mut ws = server.connect_ws(&ada_cookie).await;
        open_wavelet(&mut ws, &root, &doc).await;
        ws.send(update_frame(&root, edit(&doc, "before disconnect. ")))
            .await
            .unwrap();
        ws.close(None).await.unwrap();
    }

    // While "offline", another session edits the wavelet...
    {
        let other = Doc::new();
        let mut ws = server.connect_ws(&ada_cookie).await;
        open_wavelet(&mut ws, &root, &other).await;
        assert_eq!(body_text(&other), "before disconnect. ");
        ws.send(update_frame(&root, edit(&other, "while away. ")))
            .await
            .unwrap();
        ws.close(None).await.unwrap();
    }

    // ...and the offline doc also edits locally.
    let offline_edit = edit(&doc, "offline edit. ");

    // Reconnect: subscribe with our state vector, get only the missing diff,
    // then push what the server is missing (NFR-21).
    let mut ws = server.connect_ws(&ada_cookie).await;
    open_wavelet(&mut ws, &root, &doc).await;
    ws.send(update_frame(&root, offline_edit)).await.unwrap();

    let text = body_text(&doc);
    assert!(text.contains("before disconnect"));
    assert!(text.contains("while away"));
    assert!(text.contains("offline edit"));

    // A fresh client sees all three (server merged everything).
    let fresh = Doc::new();
    let mut ws2 = server.connect_ws(&ada_cookie).await;
    open_wavelet(&mut ws2, &root, &fresh).await;
    let fresh_text = body_text(&fresh);
    assert!(fresh_text.contains("before disconnect"));
    assert!(fresh_text.contains("while away"));
    assert!(fresh_text.contains("offline edit"));
}

#[tokio::test]
async fn awareness_relays_and_replays_to_late_joiners() {
    let server = spawn().await;
    let ada_cookie = server.register("ada").await;
    let bob_cookie = server.register("bob").await;
    let (wave, root) = create_wave(&server, &ada_cookie, "presence").await;
    server
        .post(
            "/api/waves/participants",
            Some(&ada_cookie),
            serde_json::json!({ "wave": wave, "participant": "bob@localhost" }),
        )
        .await;

    let mut ada_ws = server.connect_ws(&ada_cookie).await;
    let doc = Doc::new();
    open_wavelet(&mut ada_ws, &root, &doc).await;
    ada_ws
        .send(awareness_frame(&root, b"ada-cursor".to_vec()))
        .await
        .unwrap();

    // Late joiner receives the cached awareness payload.
    let mut bob_ws = server.connect_ws(&bob_cookie).await;
    let bob_doc = Doc::new();
    let sv = bob_doc.transact().state_vector().encode_v1();
    bob_ws.send(subscribe_frame(&root, sv)).await.unwrap();
    let mut got_awareness = false;
    for _ in 0..4 {
        if let Inbound::Awareness(p) = recv(&mut bob_ws).await {
            assert_eq!(p, b"ada-cursor");
            got_awareness = true;
            break;
        }
    }
    assert!(got_awareness, "late joiner should get cached awareness");
}

#[tokio::test]
async fn non_participant_rejected() {
    let server = spawn().await;
    let ada_cookie = server.register("ada").await;
    let carol_cookie = server.register("carol").await;
    let (_wave, root) = create_wave(&server, &ada_cookie, "private").await;

    let mut ws = server.connect_ws(&carol_cookie).await;
    ws.send(subscribe_frame(&root, vec![])).await.unwrap();
    match recv(&mut ws).await {
        Inbound::Error(code) => assert_eq!(code, "not-participant"),
        _ => panic!("expected rejection"),
    }
}

#[tokio::test]
async fn unauthenticated_ws_rejected() {
    let server = spawn().await;
    let req = server.ws_url.clone().into_client_request().unwrap();
    let err = tokio_tungstenite::connect_async(req).await;
    assert!(err.is_err(), "ws without session cookie must be rejected");
}

#[tokio::test]
async fn auth_flow_and_wrong_password() {
    let server = spawn().await;
    let cookie = server.register("ada").await;

    // /api/me works with the cookie.
    let req = Request::get("/api/me")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let res = server.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Wrong password rejected.
    let (status, _, _) = server
        .post(
            "/api/login",
            None,
            serde_json::json!({ "name": "ada", "password": "wrong password" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Correct password issues a session.
    let (status, new_cookie, _) = server
        .post(
            "/api/login",
            None,
            serde_json::json!({ "name": "ada", "password": "correct horse battery" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(new_cookie.is_some());
}
