//! Phase 7 (exploratory): wave agents / Hive Mind (PRD §12.1). Verifies the
//! harness — an agent authors a real blip into the shared doc — and the
//! signed federated-inference path (mixture-of-peers), using a mock model.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::util::ServiceExt;
use yrs::{Any, Array, Doc, Out, Transact};

use protowave_core::WaveletName;
use protowave_server::agent::InferenceProvider;
use protowave_server::federation::FederationConfig;
use protowave_server::store::FileStore;
use protowave_server::{app, AppState};

struct MockModel;

#[async_trait]
impl InferenceProvider for MockModel {
    async fn infer(&self, prompt: &str, context: &str) -> io::Result<String> {
        // Echo enough to prove RAG context reached the model.
        let grounded = context.contains("lighthouse");
        Ok(format!(
            "Re: {prompt}\n{}",
            if grounded {
                "I found the lighthouse note on this wave."
            } else {
                "No local context found."
            }
        ))
    }
    fn model(&self) -> String {
        "mock-1".into()
    }
}

struct Server {
    router: axum::Router,
    state: Arc<AppState>,
    _dir: tempfile::TempDir,
}

async fn spawn(domain: &str, peers: HashMap<String, String>, with_model: bool) -> (Server, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let state = AppState::build(
        store,
        domain,
        dir.path(),
        false,
        FederationConfig {
            public_url: url.clone(),
            peers,
        },
    )
    .unwrap();
    if with_model {
        state.inference.set(Arc::new(MockModel));
    }
    let router = app(state.clone());
    let serve = router.clone();
    tokio::spawn(async move { axum::serve(listener, serve).await.unwrap() });
    (
        Server {
            router,
            state,
            _dir: dir,
        },
        url,
    )
}

impl Server {
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

    async fn register(&self, name: &str) -> String {
        let (s, c, _) = self
            .json(
                "/api/register",
                None,
                serde_json::json!({ "name": name, "password": "correct horse battery" }),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        c.unwrap()
    }
}

/// Author + text of every manifest blip, read from a materialized doc.
fn blip_authors(doc: &Doc) -> Vec<String> {
    let manifest = doc.get_or_insert_array("manifest");
    let txn = doc.transact();
    let mut out = Vec::new();
    for i in 0..manifest.len(&txn) {
        if let Some(Out::Any(Any::Map(map))) = manifest.get(&txn, i) {
            if let Some(Any::String(author)) = map.get("author") {
                out.push(author.to_string());
            }
        }
    }
    out
}

async fn materialize(server: &Server, wavelet: &str) -> Doc {
    let name: WaveletName = wavelet.parse().unwrap();
    let live = server.state.engine.open_wavelet(&name).await.unwrap();
    let (_sv, diff) = live.sync_state(&[]).unwrap();
    let doc = Doc::new();
    if !diff.is_empty() {
        use yrs::updates::decoder::Decode;
        doc.transact_mut()
            .apply_update(yrs::Update::decode_v1(&diff).unwrap());
    }
    doc
}

#[tokio::test]
async fn ollama_provider_calls_the_local_api() {
    use axum::routing::post;
    use axum::{Json, Router};
    use protowave_server::agent::{InferenceProvider, OllamaInference};

    // A stand-in Ollama server: echoes the model and confirms the prompt
    // carried our context, mimicking POST /api/generate.
    async fn generate(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
        let prompt = body["prompt"].as_str().unwrap_or("");
        let grounded = prompt.contains("harbor");
        Json(serde_json::json!({
            "model": body["model"],
            "response": if grounded { "The harbor plan is on track." } else { "no context" },
            "done": true,
        }))
    }
    let app = Router::new().route("/api/generate", post(generate));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let provider = OllamaInference::new(format!("http://{addr}"), "gemma3:270m".into());
    assert_eq!(provider.model(), "ollama/gemma3:270m");
    let answer = provider
        .infer("what about the harbor?", "notes about the harbor")
        .await
        .unwrap();
    assert_eq!(answer, "The harbor plan is on track.");
}

#[tokio::test]
async fn agent_answers_as_a_blip_with_context() {
    let (server, _url) = spawn("localhost", HashMap::new(), true).await;
    let ada = server.register("ada").await;
    let (_s, _c, wave) = server
        .json(
            "/api/waves",
            Some(&ada),
            serde_json::json!({ "title": "lighthouse plans" }),
        )
        .await;
    let wave_id = wave["wave"].as_str().unwrap().to_string();
    let root = wave["rootWavelet"].as_str().unwrap().to_string();

    // Give the search indexer content to retrieve (title has "lighthouse").
    // Poll until indexed, then ask.
    let mut answer = serde_json::Value::Null;
    for _ in 0..40 {
        let (status, _, resp) = server
            .json(
                "/api/waves/ask",
                Some(&ada),
                serde_json::json!({ "wave": wave_id, "prompt": "what about the lighthouse?" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        answer = resp;
        if answer["answer"]
            .as_str()
            .unwrap_or("")
            .contains("lighthouse note")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(answer["model"], "mock-1");
    assert!(answer["agent"].as_str().unwrap().starts_with("assistant@"));

    // The answer is a real blip authored by the agent in the shared doc.
    let doc = materialize(&server, &root).await;
    let authors = blip_authors(&doc);
    assert!(
        authors.iter().any(|a| a == "assistant@localhost"),
        "agent blip missing; authors = {authors:?}"
    );
}

#[tokio::test]
async fn mentioning_the_assistant_triggers_a_reply_once() {
    let (server, _url) = spawn("localhost", HashMap::new(), true).await;
    let ada = server.register("ada").await;
    let (_s, _c, wave) = server
        .json(
            "/api/waves",
            Some(&ada),
            serde_json::json!({ "title": "harbor" }),
        )
        .await;
    let root = wave["rootWavelet"].as_str().unwrap().to_string();

    // ada writes a blip mentioning @assistant, through the engine.
    let name: WaveletName = root.parse().unwrap();
    let live = server.state.engine.open_wavelet(&name).await.unwrap();
    let update = protowave_server::agent::agent_blip_update(
        &Doc::new(),
        "ada@localhost",
        "hey @assistant, what about the harbor?",
        None,
    );
    server
        .state
        .engine
        .apply_update(&live, update, 1)
        .await
        .unwrap();

    // The mention worker (2.5s debounce) posts one agent reply.
    let mut agent_replies = 0;
    for _ in 0..80 {
        let doc = materialize(&server, &root).await;
        agent_replies = blip_authors(&doc)
            .iter()
            .filter(|a| *a == "assistant@localhost")
            .count();
        if agent_replies >= 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(agent_replies, 1, "exactly one agent reply expected");

    // Give the worker more time — it must NOT answer the same mention again
    // (and must not answer its own reply, which would loop).
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    let doc = materialize(&server, &root).await;
    let again = blip_authors(&doc)
        .iter()
        .filter(|a| *a == "assistant@localhost")
        .count();
    assert_eq!(again, 1, "no duplicate / no self-triggered loop");
}

#[tokio::test]
async fn ask_requires_participant_and_model() {
    // No model configured → 503.
    let (server, _url) = spawn("localhost", HashMap::new(), false).await;
    let ada = server.register("ada").await;
    let (_s, _c, wave) = server
        .json(
            "/api/waves",
            Some(&ada),
            serde_json::json!({ "title": "t" }),
        )
        .await;
    let wave_id = wave["wave"].as_str().unwrap().to_string();
    let (status, _, _) = server
        .json(
            "/api/waves/ask",
            Some(&ada),
            serde_json::json!({ "wave": wave_id, "prompt": "hi" }),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    // Non-participant → 403.
    let eve = server.register("eve").await;
    let (status, _, _) = server
        .json(
            "/api/waves/ask",
            Some(&eve),
            serde_json::json!({ "wave": wave_id, "prompt": "hi" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn federated_inference_routes_to_a_peer_model() {
    // A hosts the model; B has none. A wave on A includes bob@b.local, so
    // B may call A's model over the signed federation channel (FI).
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let la = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let lb = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url_a = format!("http://{}", la.local_addr().unwrap());
    let url_b = format!("http://{}", lb.local_addr().unwrap());

    let state_a = AppState::build(
        Arc::new(FileStore::open(dir_a.path(), false).unwrap()),
        "a.local",
        dir_a.path(),
        false,
        FederationConfig {
            public_url: url_a.clone(),
            peers: [("b.local".to_string(), url_b.clone())].into(),
        },
    )
    .unwrap();
    state_a.inference.set(Arc::new(MockModel)); // A hosts the model
    let state_b = AppState::build(
        Arc::new(FileStore::open(dir_b.path(), false).unwrap()),
        "b.local",
        dir_b.path(),
        false,
        FederationConfig {
            public_url: url_b.clone(),
            peers: [("a.local".to_string(), url_a.clone())].into(),
        },
    )
    .unwrap();

    let ra = app(state_a.clone());
    let rb = app(state_b.clone());
    let (sa, sb) = (ra.clone(), rb.clone());
    tokio::spawn(async move { axum::serve(la, sa).await.unwrap() });
    tokio::spawn(async move { axum::serve(lb, sb).await.unwrap() });

    // ada@a.local creates a wave and adds bob@b.local.
    let reg = |router: axum::Router, name: &'static str| async move {
        let res = router
            .oneshot(
                Request::post("/api/register")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"name":"{name}","password":"correct horse battery"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        res.headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    };
    let ada = reg(ra.clone(), "ada").await;
    let _bob = reg(rb.clone(), "bob").await;

    let res = ra
        .clone()
        .oneshot(
            Request::post("/api/waves")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, &ada)
                .body(Body::from(r#"{"title":"cross-node"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let wave: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let wave_id = wave["wave"].as_str().unwrap().to_string();

    ra.clone()
        .oneshot(
            Request::post("/api/waves/participants")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, &ada)
                .body(Body::from(format!(
                    r#"{{"wave":"{wave_id}","participant":"bob@b.local"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    // Wait for the announcement so A knows b.local participates (already
    // does, it's the home server) and B knows the wave.
    for _ in 0..50 {
        if state_b.store.get_wave(&wave_id).await.unwrap().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // B has no local model, so it routes inference to A over federation.
    let (text, model) =
        protowave_server::federation::peer_infer(&state_b, "a.local", &wave_id, "ping", "ctx")
            .await
            .expect("federated inference should succeed");
    assert_eq!(model, "mock-1");
    assert!(text.contains("Re: ping"));
}
