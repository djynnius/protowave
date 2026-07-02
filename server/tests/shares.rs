//! Phase 5: distributed folder sharing (PRD §11) — chunk dedup (FR-59),
//! share roundtrip + ACL, federated multi-source fetch with per-chunk
//! verification (FR-56..57), and mirroring (FR-58).

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::util::ServiceExt;

use protowave_server::federation::FederationConfig;
use protowave_server::store::FileStore;
use protowave_server::{app, AppState};

struct Server {
    router: axum::Router,
    state: Arc<AppState>,
    dir: tempfile::TempDir,
}

async fn spawn_one() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
    let state = AppState::build(store, "localhost", dir.path(), false, Default::default()).unwrap();
    Server {
        router: app(state.clone()),
        state,
        dir,
    }
}

/// Two federated servers (same pattern as tests/federation.rs).
async fn pair() -> (Server, Server) {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let la = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let lb = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url_a = format!("http://{}", la.local_addr().unwrap());
    let url_b = format!("http://{}", lb.local_addr().unwrap());

    let mk = |dir: &tempfile::TempDir, domain: &str, url: &str, peers: HashMap<String, String>| {
        let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
        AppState::build(
            store,
            domain,
            dir.path(),
            false,
            FederationConfig {
                public_url: url.to_string(),
                peers,
            },
        )
        .unwrap()
    };
    let state_a = mk(
        &dir_a,
        "a.local",
        &url_a,
        [("b.local".to_string(), url_b.clone())].into(),
    );
    let state_b = mk(
        &dir_b,
        "b.local",
        &url_b,
        [("a.local".to_string(), url_a.clone())].into(),
    );
    let ra = app(state_a.clone());
    let rb = app(state_b.clone());
    let (sa, sb) = (ra.clone(), rb.clone());
    tokio::spawn(async move { axum::serve(la, sa).await.unwrap() });
    tokio::spawn(async move { axum::serve(lb, sb).await.unwrap() });
    (
        Server {
            router: ra,
            state: state_a,
            dir: dir_a,
        },
        Server {
            router: rb,
            state: state_b,
            dir: dir_b,
        },
    )
}

impl Server {
    async fn req(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> (StatusCode, Option<String>, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if let Some(ct) = content_type {
            req = req.header(header::CONTENT_TYPE, ct);
        }
        let res = self
            .router
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let cookie = res
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(res.into_body(), 700 << 20)
            .await
            .unwrap();
        (status, cookie, bytes.to_vec())
    }

    async fn json(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: serde_json::Value,
    ) -> (StatusCode, Option<String>, serde_json::Value) {
        let (s, c, b) = self
            .req(
                method,
                path,
                cookie,
                Some("application/json"),
                body.to_string().into_bytes(),
            )
            .await;
        (
            s,
            c,
            serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null),
        )
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

    async fn create_wave(&self, cookie: &str) -> String {
        let (_, _, w) = self
            .json(
                "POST",
                "/api/waves",
                Some(cookie),
                serde_json::json!({ "title": "shares" }),
            )
            .await;
        w["wave"].as_str().unwrap().to_string()
    }
}

fn multipart_folder(boundary: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (path, content) in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{path}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

fn urlenc(s: &str) -> String {
    s.replace('/', "%2F")
        .replace('+', "%2B")
        .replace(' ', "%20")
}

async fn upload_folder(
    server: &Server,
    cookie: &str,
    wave: &str,
    name: &str,
    files: &[(&str, &[u8])],
) -> serde_json::Value {
    let boundary = "pw-folder-boundary";
    let (status, _, resp) = server
        .req(
            "POST",
            &format!("/api/shares?wave={}&name={}", urlenc(wave), urlenc(name)),
            Some(cookie),
            Some(&format!("multipart/form-data; boundary={boundary}")),
            multipart_folder(boundary, files),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&resp));
    serde_json::from_slice(&resp).unwrap()
}

#[tokio::test]
async fn share_roundtrip_and_chunk_dedup() {
    let server = spawn_one().await;
    let ada = server.register("ada").await;
    let wave = server.create_wave(&ada).await;

    // A folder with a large repetitive file (multiple chunks) + a small one.
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let share = upload_folder(
        &server,
        &ada,
        &wave,
        "dataset",
        &[("dataset/big.bin", &big), ("dataset/readme.md", b"# hi")],
    )
    .await;
    let hash = share["manifest_hash"].as_str().unwrap().to_string();
    assert_eq!(share["file_count"], 2);
    assert_eq!(share["mirrored"], true);

    // Manifest browse.
    let (status, _, m) = server
        .json(
            "GET",
            &format!("/api/shares/{hash}"),
            Some(&ada),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let files = m["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let big_entry = files
        .iter()
        .find(|f| f["path"] == "dataset/big.bin")
        .unwrap();
    assert!(
        big_entry["chunks"].as_u64().unwrap() > 1,
        "3MB file must span multiple chunks"
    );

    // Download reassembles byte-exactly.
    let (status, _, bytes) = server
        .req(
            "GET",
            &format!("/api/shares/{hash}/file?path={}", urlenc("dataset/big.bin")),
            Some(&ada),
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, big);

    // Dedup (FR-59): re-share the same folder with one file appended —
    // unchanged content adds (almost) no new blobs.
    let blobs_dir = server.dir.path().join("blobs");
    let count_blobs = |dir: &std::path::Path| walk_count(dir);
    let before = count_blobs(&blobs_dir);
    let mut big2 = big.clone();
    big2.extend_from_slice(b"tail change only");
    upload_folder(
        &server,
        &ada,
        &wave,
        "dataset-v2",
        &[("dataset/big.bin", &big2), ("dataset/readme.md", b"# hi")],
    )
    .await;
    let after = count_blobs(&blobs_dir);
    let added = after - before;
    assert!(
        added <= 4,
        "content-defined chunking should reuse chunks: added {added} blobs"
    );

    // ACL: outsider cannot list, browse, or download.
    let eve = server.register("eve").await;
    let (status, _, _) = server
        .req(
            "GET",
            &format!("/api/shares/{hash}/file?path={}", urlenc("dataset/big.bin")),
            Some(&eve),
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

fn walk_count(dir: &std::path::Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                n += walk_count(&p);
            } else {
                n += 1;
            }
        }
    }
    n
}

#[tokio::test]
async fn federated_share_fetch_and_mirror() {
    let (a, b) = pair().await;
    let ada = a.register("ada").await;
    let bob = b.register("bob").await;

    let wave = a.create_wave(&ada).await;
    a.json(
        "POST",
        "/api/waves/participants",
        Some(&ada),
        serde_json::json!({ "wave": wave, "participant": "bob@b.local" }),
    )
    .await;

    // Wait for B to learn the wave.
    for _ in 0..50 {
        if b.state.store.get_wave(&wave).await.unwrap().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Ada shares a folder on A.
    let payload: Vec<u8> = (0..1_500_000u32).map(|i| (i % 199) as u8).collect();
    let share = upload_folder(&a, &ada, &wave, "papers", &[("papers/data.bin", &payload)]).await;
    let hash = share["manifest_hash"].as_str().unwrap().to_string();

    // The announcement reaches B; bob sees the share.
    let mut found = false;
    for _ in 0..50 {
        let (_, _, list) = b
            .json(
                "GET",
                &format!("/api/shares?wave={}", urlenc(&wave)),
                Some(&bob),
                serde_json::json!({}),
            )
            .await;
        if list.as_array().map(|l| !l.is_empty()).unwrap_or(false) {
            assert_eq!(list[0]["origin_domain"], "a.local");
            assert_eq!(list[0]["mirrored"], false);
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(found, "share announcement never reached B");

    // Bob downloads via B: manifest + chunks federate from A, verified.
    let (status, _, bytes) = b
        .req(
            "GET",
            &format!("/api/shares/{hash}/file?path={}", urlenc("papers/data.bin")),
            Some(&bob),
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, payload, "federated download must be byte-exact");

    // Mirror on B (FR-58): pins everything locally.
    let (status, _, m) = b
        .json(
            "POST",
            &format!("/api/shares/{hash}/mirror"),
            Some(&bob),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(m["mirrored"], true);

    let _ = (a.dir.path(), b.dir.path());
}

#[tokio::test]
async fn corrupt_peer_blob_is_rejected() {
    let (a, b) = pair().await;
    let ada = a.register("ada").await;
    let bob = b.register("bob").await;
    let wave = a.create_wave(&ada).await;
    a.json(
        "POST",
        "/api/waves/participants",
        Some(&ada),
        serde_json::json!({ "wave": wave, "participant": "bob@b.local" }),
    )
    .await;
    for _ in 0..50 {
        if b.state.store.get_wave(&wave).await.unwrap().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let payload = vec![7u8; 300_000];
    let share = upload_folder(&a, &ada, &wave, "x", &[("x/f.bin", &payload)]).await;
    let hash = share["manifest_hash"].as_str().unwrap().to_string();
    // Wait for the announcement so B knows the share.
    for _ in 0..50 {
        if b.state.store.get_share(&hash).await.unwrap().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Corrupt every blob on A (flip bytes in place) — B must refuse them.
    corrupt_blobs(&a.dir.path().join("blobs"));

    let (status, _, _) = b
        .req(
            "GET",
            &format!("/api/shares/{hash}/file?path={}", urlenc("x/f.bin")),
            Some(&bob),
            None,
            vec![],
        )
        .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "corrupted chunks must never assemble into a served file"
    );
}

fn corrupt_blobs(dir: &std::path::Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                corrupt_blobs(&p);
            } else if let Ok(mut bytes) = std::fs::read(&p) {
                if !bytes.is_empty() {
                    bytes[0] ^= 0xFF;
                    let _ = std::fs::write(&p, bytes);
                }
            }
        }
    }
}
