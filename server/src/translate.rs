//! Live translation engine (PRD §9, FR-40..46).
//!
//! Principles: translations are **ephemeral overlays** keyed by blip
//! content — never stored in the document; work is O(changed blips)
//! (NFR-C7); provider is swappable behind `Translator` (FR-46); nothing
//! leaves the server unless the wave opted in (FR-40, NFR-16).
//!
//! Flow: a client on a translation-enabled wave sends TranslateSubscribe
//! with a target language. The hub listens to the engine's change stream;
//! on each (debounced) change it re-extracts blip text, translates blips
//! whose content hash isn't cached for that language, and sends
//! TranslationMessage frames to every subscribed connection. Because the
//! prompt carries the preceding blips as context and the trailing blip
//! re-translates while it grows, the overlay revises itself as meaning
//! accumulates (US-6) and freezes (via cache) once the text stabilizes.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::TokioExecutor;
use prost::Message;
use tokio::sync::mpsc;

use protowave_core::WaveletName;
use protowave_proto::v1 as pb;

use crate::AppState;

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

/// A translation provider (FR-46). Implementations must be safe to call
/// concurrently.
#[async_trait]
pub trait Translator: Send + Sync + 'static {
    /// Translate `text` into `target_lang`. `context` is preceding
    /// conversation content the provider may use for disambiguation.
    async fn translate(&self, target_lang: &str, context: &str, text: &str) -> io::Result<String>;
}

// ---------------------------------------------------------------------------
// Gemini reference implementation
// ---------------------------------------------------------------------------

type HttpsClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>;

pub struct GeminiTranslator {
    key: String,
    model: String,
    client: HttpsClient,
}

impl GeminiTranslator {
    pub fn new(key: String, model: String) -> Self {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_http1()
            .build();
        Self {
            key,
            model,
            client: Client::builder(TokioExecutor::new()).build(https),
        }
    }
}

#[async_trait]
impl Translator for GeminiTranslator {
    async fn translate(&self, target_lang: &str, context: &str, text: &str) -> io::Result<String> {
        let prompt = format!(
            "You are a live translation engine inside a collaborative tool. \
             Translate the MESSAGE into the language with code {target_lang:?}. \
             Preserve tone and meaning; keep names and code as-is. \
             The message may be an unfinished sentence mid-typing — translate \
             what is there naturally. Reply with ONLY the translation.\n\
             CONVERSATION CONTEXT (for disambiguation only):\n{context}\n\
             MESSAGE:\n{text}"
        );
        let body = serde_json::json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": { "temperature": 0.1 }
        })
        .to_string();

        let uri: hyper::Uri = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        )
        .parse()
        .map_err(other)?;
        let req = hyper::Request::post(uri)
            .header("content-type", "application/json")
            .header("x-goog-api-key", &self.key)
            .body(Full::new(Bytes::from(body)))
            .map_err(other)?;
        let res = self.client.request(req).await.map_err(other)?;
        let status = res.status();
        let bytes = res.into_body().collect().await.map_err(other)?.to_bytes();
        if !status.is_success() {
            return Err(other(format!(
                "gemini: {status} {}",
                String::from_utf8_lossy(&bytes[..bytes.len().min(200)])
            )));
        }
        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(other)?;
        json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .map(|s| s.trim().to_string())
            .ok_or_else(|| other("gemini: no candidate text"))
    }
}

// ---------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------

struct Subscription {
    lang: String,
    out: mpsc::Sender<Vec<u8>>,
}

pub struct TranslationHub {
    translator: RwLock<Option<Arc<dyn Translator>>>,
    /// (content hash, lang) → translation. Content-keyed, so a stabilized
    /// blip is never re-translated (FR-43) and replay/rejoin is free.
    cache: Mutex<HashMap<(u64, String), String>>,
    /// wavelet → conn id → subscription.
    subs: Mutex<HashMap<String, HashMap<u64, Subscription>>>,
    calls: AtomicU64,
    call_cap: u64,
}

fn content_hash(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

impl TranslationHub {
    pub fn new(call_cap: u64) -> Self {
        Self {
            translator: RwLock::new(None),
            cache: Mutex::new(HashMap::new()),
            subs: Mutex::new(HashMap::new()),
            calls: AtomicU64::new(0),
            call_cap,
        }
    }

    pub fn set_translator(&self, t: Arc<dyn Translator>) {
        *self.translator.write().unwrap() = Some(t);
    }

    pub fn available(&self) -> bool {
        self.translator.read().unwrap().is_some()
    }

    pub fn subscribe(&self, wavelet: &str, conn: u64, lang: String, out: mpsc::Sender<Vec<u8>>) {
        self.subs
            .lock()
            .unwrap()
            .entry(wavelet.to_string())
            .or_default()
            .insert(conn, Subscription { lang, out });
    }

    pub fn unsubscribe(&self, wavelet: &str, conn: u64) {
        let mut subs = self.subs.lock().unwrap();
        if let Some(conns) = subs.get_mut(wavelet) {
            conns.remove(&conn);
            if conns.is_empty() {
                subs.remove(wavelet);
            }
        }
    }

    pub fn drop_conn(&self, conn: u64) {
        let mut subs = self.subs.lock().unwrap();
        subs.retain(|_, conns| {
            conns.remove(&conn);
            !conns.is_empty()
        });
    }

    fn languages_for(&self, wavelet: &str) -> Vec<String> {
        let subs = self.subs.lock().unwrap();
        let mut langs: Vec<String> = subs
            .get(wavelet)
            .map(|conns| conns.values().map(|s| s.lang.clone()).collect())
            .unwrap_or_default();
        langs.sort();
        langs.dedup();
        langs
    }

    fn deliver(&self, wavelet: &str, lang: &str, frame: Vec<u8>) {
        let subs = self.subs.lock().unwrap();
        if let Some(conns) = subs.get(wavelet) {
            for sub in conns.values().filter(|s| s.lang == lang) {
                let _ = sub.out.try_send(frame.clone());
            }
        }
    }
}

/// Translate every blip of a wavelet into every subscribed language and
/// deliver overlay frames. Cache hits cost nothing; misses call the
/// provider under the call budget (FR-44).
pub async fn translate_wavelet(state: &Arc<AppState>, name: &WaveletName) {
    let hub = &state.translation;
    let langs = hub.languages_for(&name.to_string());
    if langs.is_empty() {
        return;
    }
    let translator = match hub.translator.read().unwrap().clone() {
        Some(t) => t,
        None => return,
    };
    let live = match state.engine.open_wavelet(name).await {
        Ok(live) => live,
        Err(_) => return,
    };
    let blips = live.extract_blips();
    let full_context: String = blips
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    for lang in langs {
        let mut entries = Vec::new();
        for (blip, text) in &blips {
            let key = (content_hash(text), lang.clone());
            let cached = hub.cache.lock().unwrap().get(&key).cloned();
            let translated = match cached {
                Some(t) => t,
                None => {
                    if hub.calls.fetch_add(1, Ordering::Relaxed) >= hub.call_cap {
                        tracing::warn!("translation call budget exhausted (FR-44)");
                        continue;
                    }
                    match translator.translate(&lang, &full_context, text).await {
                        Ok(t) => {
                            hub.cache.lock().unwrap().insert(key, t.clone());
                            t
                        }
                        Err(e) => {
                            tracing::warn!(%e, "translation failed");
                            continue;
                        }
                    }
                }
            };
            entries.push(pb::TranslationEntry {
                blip: blip.clone(),
                text: translated,
                pending: false,
            });
        }
        if entries.is_empty() {
            continue;
        }
        let frame = pb::Envelope::new(
            pb::Channel::Translation,
            pb::TranslationMessage {
                wavelet: name.to_string(),
                target_lang: lang.clone(),
                entries,
            }
            .encode_to_vec(),
        )
        .encode_frame();
        hub.deliver(&name.to_string(), &lang, frame);
    }
}

/// Background worker: debounced re-translation on document change.
pub fn spawn_translation_worker(state: Arc<AppState>) {
    let mut rx = state.engine.change_stream();
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Debounce: let a typing burst settle, then coalesce.
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            let mut batch = std::collections::HashSet::new();
            batch.insert(first);
            while let Ok(more) = rx.try_recv() {
                batch.insert(more);
            }
            for name in batch {
                translate_wavelet(&state, &name).await;
            }
        }
    });
}

// Compiled unconditionally so integration tests can inject it.
#[doc(hidden)]
pub mod testing {
    use super::*;

    /// Deterministic mock: "[lang] original text", counts provider calls.
    pub struct MockTranslator {
        pub calls: std::sync::atomic::AtomicUsize,
    }

    impl Default for MockTranslator {
        fn default() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl Translator for MockTranslator {
        async fn translate(
            &self,
            target_lang: &str,
            _context: &str,
            text: &str,
        ) -> io::Result<String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(format!("[{target_lang}] {text}"))
        }
    }
}
