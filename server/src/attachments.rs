//! Attachment upload/download (FR-35..37).
//!
//! Bytes live in the BLAKE3 CAS; metadata in the WaveStore. Uploads and
//! downloads are ACL-checked against the wave's participant list.

use std::sync::Arc;

use axum::extract::{Multipart, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::auth::{ApiError, CurrentUser};
use crate::store::{now_ms, AttachmentMeta};
use crate::AppState;

const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

async fn require_participant(state: &AppState, wave: &str, user: &str) -> Result<(), ApiError> {
    let meta = state
        .store
        .get_wave(wave)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such wave".into()))?;
    if meta.participants.iter().any(|p| p == user) {
        Ok(())
    } else {
        Err(ApiError(StatusCode::FORBIDDEN, "not a participant".into()))
    }
}

#[derive(Deserialize)]
pub struct UploadQuery {
    pub wave: String,
}

pub async fn upload(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Query(q): Query<UploadQuery>,
    mut multipart: Multipart,
) -> Result<Json<AttachmentMeta>, ApiError> {
    require_participant(&state, &q.wave, &me.to_string()).await?;

    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
        .ok_or_else(|| ApiError::bad_request("no file field"))?;

    let name = field.file_name().unwrap_or("unnamed").to_string();
    let mime = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = field
        .bytes()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    if bytes.is_empty() {
        return Err(ApiError::bad_request("empty file"));
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "attachment exceeds 25 MB".into(),
        ));
    }

    let hash = state.cas.put(&bytes)?;
    let meta = AttachmentMeta {
        hash: hash.clone(),
        wave: q.wave,
        name,
        mime,
        size: bytes.len() as u64,
        uploader: me.to_string(),
        created_ms: now_ms(),
    };
    state.store.put_attachment(&meta).await?;
    tracing::info!(hash = %meta.hash, wave = %meta.wave, by = %me, "attachment stored");
    Ok(Json(meta))
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    axum::extract::Path(hash): axum::extract::Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let meta = state
        .store
        .get_attachment(&hash)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such attachment".into()))?;
    require_participant(&state, &meta.wave, &me.to_string()).await?;

    let bytes = state
        .cas
        .get(&hash)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "blob missing".into()))?;

    // Inline for previewable types, download otherwise; nosniff always
    // (NFR-17).
    let disposition = if meta.mime.starts_with("image/") || meta.mime.starts_with("text/") {
        format!("inline; filename=\"{}\"", meta.name.replace('"', ""))
    } else {
        format!("attachment; filename=\"{}\"", meta.name.replace('"', ""))
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        meta.mime
            .parse()
            .unwrap_or(header::HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        disposition
            .parse()
            .unwrap_or(header::HeaderValue::from_static("attachment")),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    // Content-addressed: safe to cache forever.
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    Ok((headers, bytes))
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub wave: String,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    CurrentUser(me): CurrentUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AttachmentMeta>>, ApiError> {
    require_participant(&state, &q.wave, &me.to_string()).await?;
    Ok(Json(state.store.list_attachments(&q.wave).await?))
}
