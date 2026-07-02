//! Wave engine (PRD §7): server-side yrs documents, sync, and fanout.
//!
//! One `LiveWavelet` per open wavelet holds the materialized yrs `Doc`,
//! a broadcast channel for subscriber fanout, and the awareness cache.
//! Complexity budgets: open is O(snapshot) + O(tail) (NFR-C3), remote batch
//! integration O(k log n) (NFR-C2), fanout O(subscribers) per event (NFR-C5).

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

use protowave_core::WaveletName;

use crate::store::{now_ms, WaveStore};

/// Snapshot every k updates so open/seek stay bounded (NFR-C3/C4, PRD §4.3).
pub const SNAPSHOT_INTERVAL: u64 = 500;

#[derive(Debug)]
pub enum EngineError {
    NotFound,
    BadPayload(String),
    Io(io::Error),
}

impl From<io::Error> for EngineError {
    fn from(e: io::Error) -> Self {
        EngineError::Io(e)
    }
}

/// Fanout event to wavelet subscribers. `from` identifies the originating
/// connection so it can skip its own events.
#[derive(Clone)]
pub struct WaveletEvent {
    pub from: u64,
    pub kind: EventKind,
}

#[derive(Clone)]
pub enum EventKind {
    Update(Arc<Vec<u8>>),
    Awareness(Arc<Vec<u8>>),
}

pub struct LiveWavelet {
    pub name: WaveletName,
    doc: Mutex<Doc>,
    update_count: AtomicU64,
    pub broadcast: broadcast::Sender<WaveletEvent>,
    /// Latest awareness payload per connection, replayed to new subscribers.
    awareness: Mutex<HashMap<u64, Arc<Vec<u8>>>>,
}

impl LiveWavelet {
    /// Server state vector + the diff covering what `client_sv` is missing.
    pub fn sync_state(&self, client_sv: &[u8]) -> Result<(Vec<u8>, Vec<u8>), EngineError> {
        let sv = if client_sv.is_empty() {
            StateVector::default()
        } else {
            StateVector::decode_v1(client_sv).map_err(|e| EngineError::BadPayload(e.to_string()))?
        };
        let doc = self.doc.lock().unwrap();
        let txn = doc.transact();
        Ok((txn.state_vector().encode_v1(), txn.encode_diff_v1(&sv)))
    }

    pub fn cached_awareness(&self) -> Vec<Arc<Vec<u8>>> {
        self.awareness.lock().unwrap().values().cloned().collect()
    }

    pub fn set_awareness(&self, conn: u64, payload: Arc<Vec<u8>>) {
        self.awareness.lock().unwrap().insert(conn, payload);
    }

    pub fn drop_awareness(&self, conn: u64) {
        self.awareness.lock().unwrap().remove(&conn);
    }
}

pub struct Engine {
    store: Arc<dyn WaveStore>,
    open: Mutex<HashMap<String, Arc<LiveWavelet>>>,
}

impl Engine {
    pub fn new(store: Arc<dyn WaveStore>) -> Self {
        Self {
            store,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Materialize a wavelet: snapshot + log tail → yrs Doc (NFR-C3).
    pub fn open_wavelet(&self, name: &WaveletName) -> Result<Arc<LiveWavelet>, EngineError> {
        let key = name.to_string();
        if let Some(live) = self.open.lock().unwrap().get(&key) {
            return Ok(live.clone());
        }

        let record = self.store.load_wavelet(name)?;
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            for bytes in record.snapshot.iter().chain(record.tail.iter()) {
                let update =
                    Update::decode_v1(bytes).map_err(|e| EngineError::BadPayload(e.to_string()))?;
                txn.apply_update(update);
            }
        }
        let (tx, _) = broadcast::channel(256);
        let live = Arc::new(LiveWavelet {
            name: name.clone(),
            doc: Mutex::new(doc),
            update_count: AtomicU64::new(record.total_updates),
            broadcast: tx,
            awareness: Mutex::new(HashMap::new()),
        });

        let mut open = self.open.lock().unwrap();
        // Another task may have opened it concurrently; keep the first.
        Ok(open.entry(key).or_insert(live).clone())
    }

    /// Apply a client update: mutate the doc, append to the durable log,
    /// snapshot every k updates, fan out to subscribers.
    pub fn apply_update(
        &self,
        live: &LiveWavelet,
        bytes: Vec<u8>,
        from: u64,
    ) -> Result<(), EngineError> {
        let update =
            Update::decode_v1(&bytes).map_err(|e| EngineError::BadPayload(e.to_string()))?;

        let snapshot = {
            let doc = live.doc.lock().unwrap();
            let mut txn = doc.transact_mut();
            txn.apply_update(update);
            drop(txn);

            let count = self.store.append_update(&live.name, &bytes)?;
            live.update_count.store(count, Ordering::SeqCst);
            if count % SNAPSHOT_INTERVAL == 0 {
                let txn = doc.transact();
                Some((
                    txn.encode_state_as_update_v1(&StateVector::default()),
                    count,
                ))
            } else {
                None
            }
        };
        if let Some((snap, covered)) = snapshot {
            self.store.save_snapshot(&live.name, &snap, covered)?;
        }
        self.store
            .touch_wave(&live.name.wave_id.to_string(), now_ms())?;

        let _ = live.broadcast.send(WaveletEvent {
            from,
            kind: EventKind::Update(Arc::new(bytes)),
        });
        Ok(())
    }

    pub fn relay_awareness(&self, live: &LiveWavelet, payload: Vec<u8>, from: u64) {
        let payload = Arc::new(payload);
        live.set_awareness(from, payload.clone());
        let _ = live.broadcast.send(WaveletEvent {
            from,
            kind: EventKind::Awareness(payload),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileStore;
    use yrs::{GetString, Text};

    fn engine() -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
        (dir, Engine::new(store))
    }

    fn text_update(doc: &Doc, text: &str) -> Vec<u8> {
        let t = doc.get_or_insert_text("t");
        let before = doc.transact().state_vector();
        {
            let mut txn = doc.transact_mut();
            let len = t.get_string(&txn).len() as u32;
            t.insert(&mut txn, len, text);
        }
        doc.transact().encode_diff_v1(&before)
    }

    #[test]
    fn concurrent_edits_converge_and_persist() {
        let (dir, engine) = engine();
        let name: WaveletName = "example.org/w+1/conv+root".parse().unwrap();
        let live = engine.open_wavelet(&name).unwrap();

        // Two independent client docs edit concurrently.
        let a = Doc::new();
        let b = Doc::new();
        engine
            .apply_update(&live, text_update(&a, "hello "), 1)
            .unwrap();
        engine
            .apply_update(&live, text_update(&b, "world"), 2)
            .unwrap();

        // A fresh client syncs from empty and sees both edits.
        let (_sv, diff) = live.sync_state(&[]).unwrap();
        let c = Doc::new();
        c.transact_mut()
            .apply_update(Update::decode_v1(&diff).unwrap());
        let merged = {
            let t = c.get_or_insert_text("t");
            let txn = c.transact();
            t.get_string(&txn)
        };
        assert!(merged.contains("hello"));
        assert!(merged.contains("world"));

        // Reopen from disk (fresh engine): same state (durability, NFR-22).
        let store = Arc::new(FileStore::open(dir.path(), false).unwrap());
        let engine2 = Engine::new(store);
        let live2 = engine2.open_wavelet(&name).unwrap();
        let (_sv2, diff2) = live2.sync_state(&[]).unwrap();
        let d = Doc::new();
        d.transact_mut()
            .apply_update(Update::decode_v1(&diff2).unwrap());
        let restored = {
            let t = d.get_or_insert_text("t");
            let txn = d.transact();
            t.get_string(&txn)
        };
        assert_eq!(restored, merged);
    }

    #[test]
    fn stale_client_gets_only_missing_diff() {
        let (_g, engine) = engine();
        let name: WaveletName = "example.org/w+2/conv+root".parse().unwrap();
        let live = engine.open_wavelet(&name).unwrap();

        let a = Doc::new();
        engine
            .apply_update(&live, text_update(&a, "one"), 1)
            .unwrap();

        // Client that already has "one" hands over its state vector...
        let (sv_after_one, diff1) = live.sync_state(&[]).unwrap();
        let client = Doc::new();
        client
            .transact_mut()
            .apply_update(Update::decode_v1(&diff1).unwrap());

        engine
            .apply_update(&live, text_update(&a, " two"), 1)
            .unwrap();

        // ...and receives a diff containing only " two".
        let (_sv, diff2) = live.sync_state(&sv_after_one).unwrap();
        client
            .transact_mut()
            .apply_update(Update::decode_v1(&diff2).unwrap());
        let t = client.get_or_insert_text("t");
        let txn = client.transact();
        assert_eq!(t.get_string(&txn), "one two");
    }
}
