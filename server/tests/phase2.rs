//! Phase 2 integration: attachments (CAS + ACL), playback history,
//! read marks, and search.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::util::ServiceExt;
use yrs::{Doc, GetString, Map, ReadTxn, Transact, XmlFragment, XmlTextPrelim};

use protowave_core::WaveletName;
use protowave_server::store::FileStore;
use protowave_server::{app, AppState};

struct TestServer {
    router: axum::Router,
    state: Arc<AppState>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> TestServer {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
    let state = AppState::build(store, "localhost", dir.path(), false).unwrap();
    TestServer {
        router: app(state.clone()),
        state,
        _dir: dir,
    }
}

impl TestServer {
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
        let bytes = axum::body::to_bytes(res.into_body(), 64 << 20)
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
        let (status, cookie, bytes) = self
            .req(
                method,
                path,
                cookie,
                Some("application/json"),
                body.to_string().into_bytes(),
            )
            .await;
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

    async fn create_wave(&self, cookie: &str, title: &str) -> (String, String) {
        let (status, _, body) = self
            .json(
                "POST",
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

    /// Apply a text edit directly through the engine (as if a client synced
    /// it), using the real document model: text lives in an XmlFragment in
    /// the `blips` map (PRD §4.3).
    async fn edit(&self, wavelet: &str, text: &str) {
        let name: WaveletName = wavelet.parse().unwrap();
        let live = self.state.engine.open_wavelet(&name).await.unwrap();
        let doc = Doc::new();
        {
            // Materialize current state first so edits stack.
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
            .apply_update(&live, update, 0)
            .await
            .unwrap();
    }
}

/// Concatenated text of the root blip fragment at the doc's current state.
fn root_blip_text(doc: &Doc) -> String {
    let blips = doc.get_or_insert_map("blips");
    let txn = doc.transact();
    match blips.get(&txn, "b+root") {
        Some(yrs::Out::YXmlFragment(f)) => f.get_string(&txn),
        _ => String::new(),
    }
}

fn multipart_body(boundary: &str, filename: &str, mime: &str, content: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

#[tokio::test]
async fn attachments_upload_download_dedup_acl() {
    let server = spawn().await;
    let ada = server.register("ada").await;
    let bob = server.register("bob").await;
    let carol = server.register("carol").await;
    let (wave, _root) = server.create_wave(&ada, "attachments").await;
    server
        .json(
            "POST",
            "/api/waves/participants",
            Some(&ada),
            serde_json::json!({ "wave": wave, "participant": "bob@localhost" }),
        )
        .await;

    // Upload as ada.
    let boundary = "pw-test-boundary";
    let body = multipart_body(boundary, "notes.md", "text/markdown", b"# Ahoy\nmarkdown!");
    let path = format!("/api/attachments?wave={}", urlencode(&wave));
    let (status, _, resp) = server
        .req(
            "POST",
            &path,
            Some(&ada),
            Some(&format!("multipart/form-data; boundary={boundary}")),
            body.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&resp));
    let meta: serde_json::Value = serde_json::from_slice(&resp).unwrap();
    let hash = meta["hash"].as_str().unwrap().to_string();
    assert_eq!(meta["name"], "notes.md");

    // Same content re-uploaded → same hash (dedup, FR-36).
    let (_, _, resp2) = server
        .req(
            "POST",
            &path,
            Some(&ada),
            Some(&format!("multipart/form-data; boundary={boundary}")),
            body,
        )
        .await;
    let meta2: serde_json::Value = serde_json::from_slice(&resp2).unwrap();
    assert_eq!(meta2["hash"].as_str().unwrap(), hash);

    // Participant bob downloads; content matches.
    let (status, _, bytes) = server
        .req(
            "GET",
            &format!("/api/attachments/{hash}"),
            Some(&bob),
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"# Ahoy\nmarkdown!");

    // Non-participant carol is rejected.
    let (status, _, _) = server
        .req(
            "GET",
            &format!("/api/attachments/{hash}"),
            Some(&carol),
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Listing shows one attachment (dedup'd).
    let (status, _, listing) = server
        .req(
            "GET",
            &format!("/api/attachments?wave={}", urlencode(&wave)),
            Some(&ada),
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let listing: serde_json::Value = serde_json::from_slice(&listing).unwrap();
    assert_eq!(listing.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn history_returns_full_log_for_playback() {
    let server = spawn().await;
    let ada = server.register("ada").await;
    let (_wave, root) = server.create_wave(&ada, "history").await;

    server.edit(&root, "first. ").await;
    server.edit(&root, "second. ").await;
    server.edit(&root, "third.").await;

    let (status, _, bytes) = server
        .req(
            "GET",
            &format!("/api/history?wavelet={}", urlencode(&root)),
            Some(&ada),
            None,
            vec![],
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Parse length-prefixed frames and replay incrementally (FR-26).
    let mut updates = Vec::new();
    let mut cursor = &bytes[..];
    while cursor.len() >= 4 {
        let len = u32::from_le_bytes(cursor[..4].try_into().unwrap()) as usize;
        updates.push(cursor[4..4 + len].to_vec());
        cursor = &cursor[4 + len..];
    }
    assert_eq!(updates.len(), 3);

    let doc = Doc::new();
    use yrs::updates::decoder::Decode;
    for (i, u) in updates.iter().enumerate() {
        doc.transact_mut()
            .apply_update(yrs::Update::decode_v1(u).unwrap());
        let text = root_blip_text(&doc);
        assert!(text.contains("first."), "update {i}: {text}");
        assert_eq!(text.contains("second."), i >= 1, "update {i}: {text}");
        assert_eq!(text.contains("third."), i >= 2, "update {i}: {text}");
    }
}

#[tokio::test]
async fn read_marks_drive_unread_badges() {
    let server = spawn().await;
    let ada = server.register("ada").await;
    let (wave, root) = server.create_wave(&ada, "unread").await;
    server.edit(&root, "activity").await;

    let (_, _, list) = server
        .json("GET", "/api/waves", Some(&ada), serde_json::json!({}))
        .await;
    assert_eq!(list[0]["unread"], true);

    server
        .json(
            "POST",
            "/api/waves/read",
            Some(&ada),
            serde_json::json!({ "wave": wave }),
        )
        .await;
    let (_, _, list) = server
        .json("GET", "/api/waves", Some(&ada), serde_json::json!({}))
        .await;
    assert_eq!(list[0]["unread"], false);
}

#[tokio::test]
async fn search_finds_wave_text_with_acl() {
    let server = spawn().await;
    let ada = server.register("ada").await;
    let eve = server.register("eve").await;
    let (_wave, root) = server.create_wave(&ada, "Voyage Plans").await;
    server
        .edit(&root, "we sail toward the lighthouse at dawn")
        .await;

    // The indexer is async; poll briefly.
    let mut hits = serde_json::Value::Null;
    for _ in 0..40 {
        let (_, _, resp) = server
            .json(
                "GET",
                "/api/search?q=lighthouse",
                Some(&ada),
                serde_json::json!({}),
            )
            .await;
        if resp.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
            hits = resp;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let arr = hits.as_array().expect("search results");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "Voyage Plans");

    // Eve is not a participant: no results.
    let (_, _, resp) = server
        .json(
            "GET",
            "/api/search?q=lighthouse",
            Some(&eve),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(resp.as_array().unwrap().len(), 0);
}

fn urlencode(s: &str) -> String {
    s.replace('/', "%2F").replace('+', "%2B")
}
