//! Persistence layer (PRD §7 storage).
//!
//! `WaveStore` is the pluggable persistence trait (successor of legacy
//! Wave's file/memory/MongoDB stores). Phase 1 ships the file-backed
//! implementation: append-only length-prefixed update logs per wavelet plus
//! JSON tables for accounts, sessions and the wave index. RocksDB/PostgreSQL
//! implement the same trait later without touching the engine.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use protowave_core::{ParticipantId, WaveletName};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub participant: String,
    /// PHC-format argon2id hash.
    pub password_hash: String,
    pub created_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveMeta {
    /// WaveId serialized form (`domain/wave-id`).
    pub wave: String,
    pub title: String,
    pub participants: Vec<String>,
    pub created_by: String,
    pub created_ms: u64,
    pub last_activity_ms: u64,
}

/// A wavelet's persisted state: latest snapshot (if any) plus the update
/// log tail not yet covered by it. Open cost is O(snapshot) + O(tail)
/// (NFR-C3); the log itself is never truncated (NFR-9).
pub struct WaveletRecord {
    pub snapshot: Option<Vec<u8>>,
    pub tail: Vec<Vec<u8>>,
    pub total_updates: u64,
}

pub trait WaveStore: Send + Sync + 'static {
    // Wavelet update logs (append-only; NFR-9).
    fn append_update(&self, wavelet: &WaveletName, update: &[u8]) -> io::Result<u64>;
    fn load_wavelet(&self, wavelet: &WaveletName) -> io::Result<WaveletRecord>;
    fn save_snapshot(
        &self,
        wavelet: &WaveletName,
        snapshot: &[u8],
        covered_updates: u64,
    ) -> io::Result<()>;

    // Accounts.
    /// Returns false (without writing) when the account already exists.
    fn create_account(&self, account: &Account) -> io::Result<bool>;
    fn get_account(&self, participant: &ParticipantId) -> io::Result<Option<Account>>;

    // Sessions.
    fn put_session(&self, session_id: &str, participant: &ParticipantId) -> io::Result<()>;
    fn get_session(&self, session_id: &str) -> io::Result<Option<ParticipantId>>;
    fn delete_session(&self, session_id: &str) -> io::Result<()>;

    // Wave index.
    fn put_wave(&self, meta: &WaveMeta) -> io::Result<()>;
    fn get_wave(&self, wave: &str) -> io::Result<Option<WaveMeta>>;
    /// Waves the participant is on, most recently active first (FR-28).
    fn list_waves_for(&self, participant: &ParticipantId) -> io::Result<Vec<WaveMeta>>;
    fn touch_wave(&self, wave: &str, at_ms: u64) -> io::Result<()>;
}

// ---------------------------------------------------------------------------
// File-backed implementation
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct Tables {
    accounts: HashMap<String, Account>,
    sessions: HashMap<String, String>,
    waves: HashMap<String, WaveMeta>,
}

pub struct FileStore {
    dir: PathBuf,
    /// Durability: fsync update appends (NFR-22). Off speeds up tests.
    fsync: bool,
    tables: Mutex<Tables>,
}

impl FileStore {
    pub fn open(dir: impl Into<PathBuf>, fsync: bool) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(dir.join("wavelets"))?;
        let tables_path = dir.join("tables.json");
        let tables = if tables_path.exists() {
            serde_json::from_slice(&fs::read(&tables_path)?)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        } else {
            Tables::default()
        };
        Ok(Self {
            dir,
            fsync,
            tables: Mutex::new(tables),
        })
    }

    /// Persist the JSON tables atomically (write + rename).
    fn flush_tables(&self, tables: &Tables) -> io::Result<()> {
        let tmp = self.dir.join("tables.json.tmp");
        let body = serde_json::to_vec_pretty(tables)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&body)?;
            if self.fsync {
                f.sync_data()?;
            }
        }
        fs::rename(tmp, self.dir.join("tables.json"))
    }

    fn wavelet_dir(&self, wavelet: &WaveletName) -> PathBuf {
        // Hex-encode the serialized name: filesystem-safe and reversible.
        self.dir
            .join("wavelets")
            .join(hex::encode(wavelet.to_string()))
    }

    fn read_log(path: &Path) -> io::Result<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };
        let mut cursor = &data[..];
        while cursor.len() >= 4 {
            let mut len_buf = [0u8; 4];
            cursor.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;
            if cursor.len() < len {
                // Torn tail write (crash mid-append): ignore the partial record.
                break;
            }
            out.push(cursor[..len].to_vec());
            cursor = &cursor[len..];
        }
        Ok(out)
    }
}

impl WaveStore for FileStore {
    fn append_update(&self, wavelet: &WaveletName, update: &[u8]) -> io::Result<u64> {
        let dir = self.wavelet_dir(wavelet);
        fs::create_dir_all(&dir)?;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("log.bin"))?;
        let mut frame = Vec::with_capacity(4 + update.len());
        frame.extend_from_slice(&(update.len() as u32).to_le_bytes());
        frame.extend_from_slice(update);
        f.write_all(&frame)?;
        if self.fsync {
            f.sync_data()?;
        }
        // Track the count in a sidecar so we don't re-scan the log.
        let count_path = dir.join("count");
        let count = fs::read_to_string(&count_path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        fs::write(count_path, count.to_string())?;
        Ok(count)
    }

    fn load_wavelet(&self, wavelet: &WaveletName) -> io::Result<WaveletRecord> {
        let dir = self.wavelet_dir(wavelet);
        let updates = Self::read_log(&dir.join("log.bin"))?;
        let total_updates = updates.len() as u64;
        let (snapshot, covered) = match fs::read(dir.join("snapshot.bin")) {
            Ok(data) if data.len() >= 8 => {
                let covered = u64::from_le_bytes(data[..8].try_into().unwrap());
                (Some(data[8..].to_vec()), covered)
            }
            _ => (None, 0),
        };
        let tail = updates.into_iter().skip(covered as usize).collect();
        Ok(WaveletRecord {
            snapshot,
            tail,
            total_updates,
        })
    }

    fn save_snapshot(
        &self,
        wavelet: &WaveletName,
        snapshot: &[u8],
        covered_updates: u64,
    ) -> io::Result<()> {
        let dir = self.wavelet_dir(wavelet);
        fs::create_dir_all(&dir)?;
        let tmp = dir.join("snapshot.bin.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&covered_updates.to_le_bytes())?;
            f.write_all(snapshot)?;
            if self.fsync {
                f.sync_data()?;
            }
        }
        fs::rename(tmp, dir.join("snapshot.bin"))
    }

    fn create_account(&self, account: &Account) -> io::Result<bool> {
        let mut tables = self.tables.lock().unwrap();
        if tables.accounts.contains_key(&account.participant) {
            return Ok(false);
        }
        tables
            .accounts
            .insert(account.participant.clone(), account.clone());
        self.flush_tables(&tables)?;
        Ok(true)
    }

    fn get_account(&self, participant: &ParticipantId) -> io::Result<Option<Account>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.accounts.get(&participant.to_string()).cloned())
    }

    fn put_session(&self, session_id: &str, participant: &ParticipantId) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables
            .sessions
            .insert(session_id.to_string(), participant.to_string());
        self.flush_tables(&tables)
    }

    fn get_session(&self, session_id: &str) -> io::Result<Option<ParticipantId>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.sessions.get(session_id).and_then(|p| p.parse().ok()))
    }

    fn delete_session(&self, session_id: &str) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables.sessions.remove(session_id);
        self.flush_tables(&tables)
    }

    fn put_wave(&self, meta: &WaveMeta) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables.waves.insert(meta.wave.clone(), meta.clone());
        self.flush_tables(&tables)
    }

    fn get_wave(&self, wave: &str) -> io::Result<Option<WaveMeta>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.waves.get(wave).cloned())
    }

    fn list_waves_for(&self, participant: &ParticipantId) -> io::Result<Vec<WaveMeta>> {
        let tables = self.tables.lock().unwrap();
        let me = participant.to_string();
        let mut waves: Vec<WaveMeta> = tables
            .waves
            .values()
            .filter(|w| w.participants.contains(&me))
            .cloned()
            .collect();
        waves.sort_by(|a, b| b.last_activity_ms.cmp(&a.last_activity_ms));
        Ok(waves)
    }

    fn touch_wave(&self, wave: &str, at_ms: u64) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        if let Some(meta) = tables.waves.get_mut(wave) {
            if at_ms > meta.last_activity_ms {
                meta.last_activity_ms = at_ms;
                self.flush_tables(&tables)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> (tempfile::TempDir, FileStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::open(dir.path(), false).unwrap();
        (dir, store)
    }

    #[test]
    fn update_log_roundtrip_and_snapshot_tail() {
        let (_g, store) = tmp_store();
        let name: WaveletName = "example.org/w+1/conv+root".parse().unwrap();
        assert_eq!(store.append_update(&name, b"u1").unwrap(), 1);
        assert_eq!(store.append_update(&name, b"u2").unwrap(), 2);
        store.save_snapshot(&name, b"snap-at-2", 2).unwrap();
        assert_eq!(store.append_update(&name, b"u3").unwrap(), 3);

        let rec = store.load_wavelet(&name).unwrap();
        assert_eq!(rec.snapshot.as_deref(), Some(&b"snap-at-2"[..]));
        assert_eq!(rec.tail, vec![b"u3".to_vec()]);
        assert_eq!(rec.total_updates, 3);
    }

    #[test]
    fn accounts_sessions_waves() {
        let (_g, store) = tmp_store();
        let ada: ParticipantId = "ada@example.org".parse().unwrap();
        let acct = Account {
            participant: ada.to_string(),
            password_hash: "phc".into(),
            created_ms: 1,
        };
        assert!(store.create_account(&acct).unwrap());
        assert!(!store.create_account(&acct).unwrap());
        assert!(store.get_account(&ada).unwrap().is_some());

        store.put_session("sid1", &ada).unwrap();
        assert_eq!(store.get_session("sid1").unwrap(), Some(ada.clone()));
        store.delete_session("sid1").unwrap();
        assert_eq!(store.get_session("sid1").unwrap(), None);

        for (i, wave) in ["example.org/w+a", "example.org/w+b"].iter().enumerate() {
            store
                .put_wave(&WaveMeta {
                    wave: wave.to_string(),
                    title: format!("wave {i}"),
                    participants: vec![ada.to_string()],
                    created_by: ada.to_string(),
                    created_ms: i as u64,
                    last_activity_ms: i as u64,
                })
                .unwrap();
        }
        store.touch_wave("example.org/w+a", 99).unwrap();
        let list = store.list_waves_for(&ada).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].wave, "example.org/w+a"); // most recent first
    }
}
