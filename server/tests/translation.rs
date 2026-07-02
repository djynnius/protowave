//! Phase 4: translation overlays (PRD §9) — opt-in enforcement (FR-40),
//! overlay delivery (FR-41..42), and content-hash caching (FR-43).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::util::ServiceExt;
use yrs::updates::encoder::Encode;
use yrs::{Doc, Map, ReadTxn, Transact, XmlFragment, XmlTextPrelim};

use protowave_proto::v1 as pb;
use protowave_proto::v1::{control_message, sync_message};
use protowave_server::store::FileStore;
use protowave_server::translate::testing::MockTranslator;
use protowave_server::{app, AppState};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct TestServer {
    router: axum::Router,
    mock: Arc<MockTranslator>,
    ws_url: String,
    _dir: tempfile::TempDir,
}

async fn spawn() -> TestServer {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
    let state = AppState::build(store, "localhost", dir.path(), false, Default::default()).unwrap();
    let mock = Arc::new(MockTranslator::default());
    state.translation.set_translator(mock.clone());
    let router = app(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = router.clone();
    tokio::spawn(async move { axum::serve(listener, serve).await.unwrap() });
    TestServer {
        router,
        mock,
        ws_url: format!("ws://{addr}/ws"),
        _dir: dir,
    }
}

impl TestServer {
    async fn json(
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
        let cookie = res
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            cookie,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    async fn setup_wave(&self, translation: bool) -> (String, String, String) {
        let (_, cookie, _) = self
            .json(
                "/api/register",
                None,
                serde_json::json!({ "name": "ada", "password": "correct horse battery" }),
            )
            .await;
        let cookie = cookie.unwrap();
        let (_, _, wave) = self
            .json(
                "/api/waves",
                Some(&cookie),
                serde_json::json!({ "title": "t" }),
            )
            .await;
        let wave_id = wave["wave"].as_str().unwrap().to_string();
        let root = wave["rootWavelet"].as_str().unwrap().to_string();
        if translation {
            let (status, _, _) = self
                .json(
                    "/api/waves/translation",
                    Some(&cookie),
                    serde_json::json!({ "wave": wave_id, "enabled": true }),
                )
                .await;
            assert_eq!(status, StatusCode::OK);
        }
        (cookie, wave_id, root)
    }

    async fn ws(&self, cookie: &str) -> Ws {
        let mut req = self.ws_url.clone().into_client_request().unwrap();
        req.headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }
}

fn frame(channel: pb::Channel, payload: Vec<u8>) -> Message {
    Message::Binary(pb::Envelope::new(channel, payload).encode_frame())
}

fn control(kind: control_message::Kind) -> Message {
    frame(
        pb::Channel::Control,
        pb::ControlMessage { kind: Some(kind) }.encode_to_vec(),
    )
}

/// Build an update writing `text` into the root blip fragment.
fn blip_edit(doc: &Doc, text: &str) -> Vec<u8> {
    let blips = doc.get_or_insert_map("blips");
    let before = doc.transact().state_vector();
    {
        let mut txn = doc.transact_mut();
        let frag = match blips.get(&txn, "b+root") {
            Some(yrs::Out::YXmlFragment(f)) => f,
            _ => blips.insert(&mut txn, "b+root", yrs::XmlFragmentPrelim::default()),
        };
        let len = frag.len(&txn);
        frag.insert(&mut txn, len, XmlTextPrelim::new(text));
    }
    doc.transact().encode_diff_v1(&before)
}

enum Inbound {
    Translation(Vec<(String, String)>),
    Error(String),
    Other,
}

async fn recv(ws: &mut Ws) -> Inbound {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
            .await
            .expect("recv timeout")
            .expect("open")
            .expect("frame");
        let bytes = match msg {
            Message::Binary(b) => b,
            _ => continue,
        };
        let env = pb::Envelope::decode_frame(&bytes).unwrap();
        match pb::Channel::try_from(env.channel).unwrap() {
            pb::Channel::Translation => {
                let msg = pb::TranslationMessage::decode(env.payload.as_slice()).unwrap();
                return Inbound::Translation(
                    msg.entries.into_iter().map(|e| (e.blip, e.text)).collect(),
                );
            }
            pb::Channel::Control => {
                let msg = pb::ControlMessage::decode(env.payload.as_slice()).unwrap();
                if let Some(control_message::Kind::Error(e)) = msg.kind {
                    return Inbound::Error(e.code);
                }
                return Inbound::Other;
            }
            _ => return Inbound::Other,
        }
    }
}

async fn open_and_edit(ws: &mut Ws, wavelet: &str, doc: &Doc, text: &str) {
    ws.send(control(control_message::Kind::Subscribe(pb::Subscribe {
        wavelet: wavelet.into(),
        state_vector: doc.transact().state_vector().encode_v1(),
    })))
    .await
    .unwrap();
    // Drain subscribed + sync-state.
    for _ in 0..2 {
        let _ = recv(ws).await;
    }
    let update = blip_edit(doc, text);
    ws.send(frame(
        pb::Channel::Sync,
        pb::SyncMessage {
            wavelet: wavelet.into(),
            kind: Some(sync_message::Kind::Update(pb::Update { update })),
        }
        .encode_to_vec(),
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn overlays_flow_and_cache_freezes() {
    let server = spawn().await;
    let (cookie, _wave, root) = server.setup_wave(true).await;
    let mut ws = server.ws(&cookie).await;
    let doc = Doc::new();
    open_and_edit(&mut ws, &root, &doc, "the tide is coming in").await;

    // Ask to read this wave in Spanish.
    ws.send(control(control_message::Kind::TranslateSubscribe(
        pb::TranslateSubscribe {
            wavelet: root.clone(),
            target_lang: "es".into(),
        },
    )))
    .await
    .unwrap();

    // Overlay arrives, produced by the mock provider.
    let entries = loop {
        match recv(&mut ws).await {
            Inbound::Translation(entries) => break entries,
            Inbound::Error(code) => panic!("unexpected error: {code}"),
            Inbound::Other => continue,
        }
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "b+root");
    assert_eq!(entries[0].1, "[es] the tide is coming in");
    let calls_after_first = server.mock.calls.load(Ordering::Relaxed);
    assert_eq!(calls_after_first, 1);

    // A second subscriber for the same language is served from cache
    // (FR-43): no new provider calls.
    let mut ws2 = server.ws(&cookie).await;
    let doc2 = Doc::new();
    ws2.send(control(control_message::Kind::Subscribe(pb::Subscribe {
        wavelet: root.clone(),
        state_vector: doc2.transact().state_vector().encode_v1(),
    })))
    .await
    .unwrap();
    for _ in 0..2 {
        let _ = recv(&mut ws2).await;
    }
    ws2.send(control(control_message::Kind::TranslateSubscribe(
        pb::TranslateSubscribe {
            wavelet: root.clone(),
            target_lang: "es".into(),
        },
    )))
    .await
    .unwrap();
    let entries = loop {
        match recv(&mut ws2).await {
            Inbound::Translation(entries) => break entries,
            _ => continue,
        }
    };
    assert_eq!(entries[0].1, "[es] the tide is coming in");
    assert_eq!(
        server.mock.calls.load(Ordering::Relaxed),
        calls_after_first,
        "cache hit must not call the provider"
    );
}

#[tokio::test]
async fn translation_requires_wave_opt_in() {
    let server = spawn().await;
    let (cookie, _wave, root) = server.setup_wave(false).await;
    let mut ws = server.ws(&cookie).await;
    let doc = Doc::new();
    open_and_edit(&mut ws, &root, &doc, "secret text").await;

    ws.send(control(control_message::Kind::TranslateSubscribe(
        pb::TranslateSubscribe {
            wavelet: root.clone(),
            target_lang: "es".into(),
        },
    )))
    .await
    .unwrap();
    let code = loop {
        match recv(&mut ws).await {
            Inbound::Error(code) => break code,
            _ => continue,
        }
    };
    assert_eq!(code, "translation-disabled");
    assert_eq!(
        server.mock.calls.load(Ordering::Relaxed),
        0,
        "no content may reach the provider without opt-in (NFR-16)"
    );
}
