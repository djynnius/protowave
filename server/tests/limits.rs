//! Phase 6: anti-abuse rate limiting (FR-63).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::util::ServiceExt;

use protowave_server::store::FileStore;
use protowave_server::{app, AppState};

#[tokio::test]
async fn wave_creation_is_rate_limited_per_user() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
    let state = AppState::build(store, "localhost", dir.path(), false, Default::default()).unwrap();
    let router = app(state);

    let register = Request::post("/api/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"name":"ada","password":"correct horse battery"}"#,
        ))
        .unwrap();
    let res = router.clone().oneshot(register).await.unwrap();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let mut last = StatusCode::CREATED;
    for i in 0..31 {
        let req = Request::post("/api/waves")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &cookie)
            .body(Body::from(format!(r#"{{"title":"wave {i}"}}"#)))
            .unwrap();
        last = router.clone().oneshot(req).await.unwrap().status();
    }
    assert_eq!(
        last,
        StatusCode::TOO_MANY_REQUESTS,
        "31st wave in an hour must trip the limiter"
    );
}
