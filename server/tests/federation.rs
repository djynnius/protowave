//! Phase 3 exit criterion (FR-53): two independent ProtoWave servers
//! federate — membership announcement, cross-server co-editing with
//! convergence, anti-entropy catch-up, and signature/ACL rejection.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::util::ServiceExt;
use yrs::{Doc, GetString, Map, ReadTxn, Transact, XmlFragment, XmlTextPrelim};

use protowave_core::WaveletName;
use protowave_server::federation::FederationConfig;
use protowave_server::store::FileStore;
use protowave_server::{app, AppState};

struct Server {
    router: axum::Router,
    state: Arc<AppState>,
    url: String,
    _dir: tempfile::TempDir,
}

/// Two servers that know each other's URLs.
async fn pair() -> (Server, Server) {
    // Chicken-and-egg on ports: bind B after A, then rebuild A's peer map
    // is impossible (config is fixed at build) — so pre-bind both listeners
    // first via two-phase spawn: A gets B's URL because we spawn B first
    // with an empty map... instead, simplest: spawn B, then A with B's URL,
    // then rebuild B with A's URL is wasteful. We cheat: bind both
    // listeners before building either state.
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let listener_a = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listener_b = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url_a = format!("http://{}", listener_a.local_addr().unwrap());
    let url_b = format!("http://{}", listener_b.local_addr().unwrap());

    let store_a = Arc::new(FileStore::open(dir_a.path(), false).unwrap());
    let store_b = Arc::new(FileStore::open(dir_b.path(), false).unwrap());
    let state_a = AppState::build(
        store_a,
        "a.local",
        dir_a.path(),
        false,
        FederationConfig {
            public_url: url_a.clone(),
            peers: [("b.local".to_string(), url_b.clone())].into(),
        },
    )
    .unwrap();
    let state_b = AppState::build(
        store_b,
        "b.local",
        dir_b.path(),
        false,
        FederationConfig {
            public_url: url_b.clone(),
            peers: [("a.local".to_string(), url_a.clone())].into(),
        },
    )
    .unwrap();

    let router_a = app(state_a.clone());
    let router_b = app(state_b.clone());
    let serve_a = router_a.clone();
    let serve_b = router_b.clone();
    tokio::spawn(async move { axum::serve(listener_a, serve_a).await.unwrap() });
    tokio::spawn(async move { axum::serve(listener_b, serve_b).await.unwrap() });

    (
        Server {
            router: router_a,
            state: state_a,
            url: url_a,
            _dir: dir_a,
        },
        Server {
            router: router_b,
            state: state_b,
            url: url_b,
            _dir: dir_b,
        },
    )
}

impl Server {
    async fn json(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: serde_json::Value,
    ) -> (StatusCode, Option<String>, serde_json::Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
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
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, cookie, json)
    }

    async fn register(&self, name: &str) -> String {
        let (status, cookie, _) = self
            .json(
                "POST",
                "/api/register",
                None,
                serde_json::json!({ "name": name, "password": "correct horse battery" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        cookie.unwrap()
    }

    /// Edit through the engine, as a locally-synced client would, then let
    /// federation push it (mirrors the ws path: apply + spawn_push_update).
    async fn edit(&self, wavelet: &str, text: &str) {
        let name: WaveletName = wavelet.parse().unwrap();
        let live = self.state.engine.open_wavelet(&name).await.unwrap();
        let doc = Doc::new();
        {
            let (_sv, diff) = live.sync_state(&[]).unwrap();
            if !diff.is_empty() {
                use yrs::updates::decoder::Decode;
                doc.transact_mut()
                    .apply_update(yrs::Update::decode_v1(&diff).unwrap());
            }
        }
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
        let update = doc.transact().encode_diff_v1(&before);
        self.state
            .engine
            .apply_update(&live, update.clone(), 1)
            .await
            .unwrap();
        protowave_server::federation::spawn_push_update(self.state.clone(), name, update);
    }

    async fn wavelet_text(&self, wavelet: &str) -> String {
        let name: WaveletName = wavelet.parse().unwrap();
        let live = self.state.engine.open_wavelet(&name).await.unwrap();
        let (_sv, diff) = live.sync_state(&[]).unwrap();
        let doc = Doc::new();
        if !diff.is_empty() {
            use yrs::updates::decoder::Decode;
            doc.transact_mut()
                .apply_update(yrs::Update::decode_v1(&diff).unwrap());
        }
        let blips = doc.get_or_insert_map("blips");
        let txn = doc.transact();
        match blips.get(&txn, "b+root") {
            Some(yrs::Out::YXmlFragment(f)) => f.get_string(&txn),
            _ => String::new(),
        }
    }
}

async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..80 {
        if check().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for: {what}");
}

#[tokio::test]
async fn membership_announcement_reaches_remote_inbox() {
    let (a, b) = pair().await;
    let ada = a.register("ada").await;
    let bob_cookie = b.register("bob").await;

    let (status, _, wave) = a
        .json(
            "POST",
            "/api/waves",
            Some(&ada),
            serde_json::json!({ "title": "Federated!" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let wave_id = wave["wave"].as_str().unwrap().to_string();

    let (status, _, _) = a
        .json(
            "POST",
            "/api/waves/participants",
            Some(&ada),
            serde_json::json!({ "wave": wave_id, "participant": "bob@b.local" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // The announcement lands on B and bob's inbox shows the wave.
    eventually("wave in bob's inbox on B", || async {
        let (_, _, list) = b
            .json(
                "GET",
                "/api/waves",
                Some(&bob_cookie),
                serde_json::json!({}),
            )
            .await;
        list.as_array().map(|a| !a.is_empty()).unwrap_or(false)
    })
    .await;

    let (_, _, list) = b
        .json(
            "GET",
            "/api/waves",
            Some(&bob_cookie),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(list[0]["wave"], wave_id.as_str());
    assert_eq!(list[0]["title"], "Federated!");
}

#[tokio::test]
async fn cross_server_edits_converge_both_ways() {
    let (a, b) = pair().await;
    let ada = a.register("ada").await;
    let _bob = b.register("bob").await;

    let (_, _, wave) = a
        .json(
            "POST",
            "/api/waves",
            Some(&ada),
            serde_json::json!({ "title": "co-edit" }),
        )
        .await;
    let wave_id = wave["wave"].as_str().unwrap().to_string();
    let root = wave["rootWavelet"].as_str().unwrap().to_string();
    a.json(
        "POST",
        "/api/waves/participants",
        Some(&ada),
        serde_json::json!({ "wave": wave_id, "participant": "bob@b.local" }),
    )
    .await;

    // Wait for B to know the wave.
    eventually("announcement on B", || async {
        b.state.store.get_wave(&wave_id).await.unwrap().is_some()
    })
    .await;

    // Ada edits on A → pushes to B.
    a.edit(&root, "from a.local! ").await;
    eventually("A's edit visible on B", || async {
        b.wavelet_text(&root).await.contains("from a.local")
    })
    .await;

    // Bob edits on B → pushes to A.
    b.edit(&root, "from b.local! ").await;
    eventually("B's edit visible on A", || async {
        a.wavelet_text(&root).await.contains("from b.local")
    })
    .await;

    // Full convergence.
    eventually("identical state", || async {
        a.wavelet_text(&root).await == b.wavelet_text(&root).await
    })
    .await;
}

#[tokio::test]
async fn anti_entropy_heals_missed_updates() {
    let (a, b) = pair().await;
    let ada = a.register("ada").await;
    let _bob = b.register("bob").await;

    let (_, _, wave) = a
        .json(
            "POST",
            "/api/waves",
            Some(&ada),
            serde_json::json!({ "title": "catch-up" }),
        )
        .await;
    let wave_id = wave["wave"].as_str().unwrap().to_string();
    let root = wave["rootWavelet"].as_str().unwrap().to_string();

    // Edits happen BEFORE bob is added — B never received pushes for them.
    a.edit(&root, "early history. ").await;

    a.json(
        "POST",
        "/api/waves/participants",
        Some(&ada),
        serde_json::json!({ "wave": wave_id, "participant": "bob@b.local" }),
    )
    .await;

    // The announcement triggers B's sync-pull, which back-fills history.
    eventually("B back-filled via anti-entropy", || async {
        b.wavelet_text(&root).await.contains("early history")
    })
    .await;
}

#[tokio::test]
async fn forged_signature_and_unknown_domain_rejected() {
    let (a, b) = pair().await;
    let ada = a.register("ada").await;
    let (_, _, wave) = a
        .json(
            "POST",
            "/api/waves",
            Some(&ada),
            serde_json::json!({ "title": "sig" }),
        )
        .await;
    let root = wave["rootWavelet"].as_str().unwrap().to_string();

    // A push claiming to be b.local but signed with the WRONG key: craft a
    // request signed by A's own key claiming b.local's identity.
    use prost::Message as _;
    let batch = protowave_proto::v1::UpdateBatch {
        wavelet: root.clone(),
        update: vec![0, 0],
        acl_version: 1,
    }
    .encode_to_vec();
    // Sign with B's real key path? No — simulate an attacker: random key.
    let attacker = protowave_core::ServerKeypair::generate();
    let msg = [
        b"protowave-fed-v0\n".as_slice(),
        b"/federation/v0/push",
        b"\n",
        &batch,
    ]
    .concat();
    let sig = attacker.sign(&msg).to_hex();

    let req = Request::post("/federation/v0/push")
        .header("x-protowave-domain", "b.local")
        .header("x-protowave-signature", sig)
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(batch.clone()))
        .unwrap();
    let res = a.router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "forged signature must be rejected"
    );

    // A domain nobody knows (no peer URL → key unresolvable) is rejected.
    let req = Request::post("/federation/v0/push")
        .header("x-protowave-domain", "evil.example")
        .header("x-protowave-signature", "00".repeat(64))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(Body::from(batch))
        .unwrap();
    let res = a.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let _ = (b.url.clone(), b.state.domain.clone());
}

#[tokio::test]
async fn non_participant_domain_rejected_even_with_valid_signature() {
    let (a, b) = pair().await;
    let ada = a.register("ada").await;
    // Wave has NO b.local participant.
    let (_, _, wave) = a
        .json(
            "POST",
            "/api/waves",
            Some(&ada),
            serde_json::json!({ "title": "private" }),
        )
        .await;
    let root = wave["rootWavelet"].as_str().unwrap().to_string();

    // B pushes with its REAL key (valid signature) but isn't on the wave.
    use prost::Message as _;
    let batch = protowave_proto::v1::UpdateBatch {
        wavelet: root.clone(),
        update: vec![0, 0],
        acl_version: 1,
    }
    .encode_to_vec();
    let err = b
        .state
        .federation
        .debug_post_signed(&b.state.domain, "a.local", "/federation/v0/push", batch)
        .await
        .expect_err("push from non-participant domain must fail");
    assert!(err.to_string().contains("403"), "got: {err}");
}
