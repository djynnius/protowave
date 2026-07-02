//! Persistence layer (PRD §7 storage).
//!
//! `WaveStore` is the pluggable persistence trait (successor of legacy
//! Wave's file/memory/MongoDB stores). Implementations: `FileStore`
//! (embedded, default) and `PgStore` (PostgreSQL, `store_pg`). Attachment
//! *blobs* are not here — they live in the filesystem CAS (`cas.rs`); the
//! store only holds their metadata.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
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
    /// WaveId serialized form (`domain/wave-id`). The wave's domain is its
    /// home server — the control-plane authority (PRD §8.3).
    pub wave: String,
    pub title: String,
    pub participants: Vec<String>,
    pub created_by: String,
    pub created_ms: u64,
    pub last_activity_ms: u64,
    /// Bumped on every membership change; federated batches carry the
    /// version they were authored under (FR-51).
    #[serde(default)]
    pub acl_version: u64,
    /// Wave-level translation opt-in (FR-40): content is only ever sent to
    /// the translation provider when this is true.
    #[serde(default)]
    pub translation_enabled: bool,
}

/// Attachment metadata (FR-37). The blob itself lives in the CAS keyed by
/// `hash` (BLAKE3 hex); identical content dedups automatically (FR-36).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub hash: String,
    pub wave: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub uploader: String,
    pub created_ms: u64,
}

/// A wavelet's persisted state: latest snapshot (if any) plus the update
/// log tail not yet covered by it. Open cost is O(snapshot) + O(tail)
/// (NFR-C3); the log itself is never truncated (NFR-9).
pub struct WaveletRecord {
    pub snapshot: Option<Vec<u8>>,
    pub tail: Vec<Vec<u8>>,
    pub total_updates: u64,
}

#[async_trait]
pub trait WaveStore: Send + Sync + 'static {
    // Wavelet update logs (append-only; NFR-9).
    async fn append_update(&self, wavelet: &WaveletName, update: &[u8]) -> io::Result<u64>;
    async fn load_wavelet(&self, wavelet: &WaveletName) -> io::Result<WaveletRecord>;
    /// The full log from the beginning — playback (FR-25..26).
    async fn read_all_updates(&self, wavelet: &WaveletName) -> io::Result<Vec<Vec<u8>>>;
    async fn save_snapshot(
        &self,
        wavelet: &WaveletName,
        snapshot: &[u8],
        covered_updates: u64,
    ) -> io::Result<()>;

    // Accounts.
    /// Returns false (without writing) when the account already exists.
    async fn create_account(&self, account: &Account) -> io::Result<bool>;
    async fn get_account(&self, participant: &ParticipantId) -> io::Result<Option<Account>>;

    // Sessions.
    async fn put_session(&self, session_id: &str, participant: &ParticipantId) -> io::Result<()>;
    async fn get_session(&self, session_id: &str) -> io::Result<Option<ParticipantId>>;
    async fn delete_session(&self, session_id: &str) -> io::Result<()>;

    // Wave index.
    async fn put_wave(&self, meta: &WaveMeta) -> io::Result<()>;
    async fn get_wave(&self, wave: &str) -> io::Result<Option<WaveMeta>>;
    /// Waves the participant is on, most recently active first (FR-28).
    async fn list_waves_for(&self, participant: &ParticipantId) -> io::Result<Vec<WaveMeta>>;
    async fn touch_wave(&self, wave: &str, at_ms: u64) -> io::Result<()>;

    // Per-user read marks (FR-8).
    async fn set_read_mark(
        &self,
        participant: &ParticipantId,
        wave: &str,
        at_ms: u64,
    ) -> io::Result<()>;
    async fn read_marks(&self, participant: &ParticipantId) -> io::Result<HashMap<String, u64>>;

    // Attachment metadata (FR-37).
    async fn put_attachment(&self, meta: &AttachmentMeta) -> io::Result<()>;
    async fn get_attachment(&self, hash: &str) -> io::Result<Option<AttachmentMeta>>;
    async fn list_attachments(&self, wave: &str) -> io::Result<Vec<AttachmentMeta>>;

    // Pinned federation peer keys, TOFU (NFR-15).
    async fn put_peer_key(&self, domain: &str, public_key_hex: &str) -> io::Result<()>;
    async fn get_peer_key(&self, domain: &str) -> io::Result<Option<String>>;
}

// ---------------------------------------------------------------------------
// File-backed implementation
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct Tables {
    accounts: HashMap<String, Account>,
    sessions: HashMap<String, String>,
    waves: HashMap<String, WaveMeta>,
    #[serde(default)]
    read_marks: HashMap<String, HashMap<String, u64>>,
    #[serde(default)]
    attachments: HashMap<String, AttachmentMeta>,
    #[serde(default)]
    peer_keys: HashMap<String, String>,
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

#[async_trait]
impl WaveStore for FileStore {
    async fn append_update(&self, wavelet: &WaveletName, update: &[u8]) -> io::Result<u64> {
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

    async fn load_wavelet(&self, wavelet: &WaveletName) -> io::Result<WaveletRecord> {
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

    async fn read_all_updates(&self, wavelet: &WaveletName) -> io::Result<Vec<Vec<u8>>> {
        Self::read_log(&self.wavelet_dir(wavelet).join("log.bin"))
    }

    async fn save_snapshot(
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

    async fn create_account(&self, account: &Account) -> io::Result<bool> {
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

    async fn get_account(&self, participant: &ParticipantId) -> io::Result<Option<Account>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.accounts.get(&participant.to_string()).cloned())
    }

    async fn put_session(&self, session_id: &str, participant: &ParticipantId) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables
            .sessions
            .insert(session_id.to_string(), participant.to_string());
        self.flush_tables(&tables)
    }

    async fn get_session(&self, session_id: &str) -> io::Result<Option<ParticipantId>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.sessions.get(session_id).and_then(|p| p.parse().ok()))
    }

    async fn delete_session(&self, session_id: &str) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables.sessions.remove(session_id);
        self.flush_tables(&tables)
    }

    async fn put_wave(&self, meta: &WaveMeta) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables.waves.insert(meta.wave.clone(), meta.clone());
        self.flush_tables(&tables)
    }

    async fn get_wave(&self, wave: &str) -> io::Result<Option<WaveMeta>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.waves.get(wave).cloned())
    }

    async fn list_waves_for(&self, participant: &ParticipantId) -> io::Result<Vec<WaveMeta>> {
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

    async fn touch_wave(&self, wave: &str, at_ms: u64) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        if let Some(meta) = tables.waves.get_mut(wave) {
            if at_ms > meta.last_activity_ms {
                meta.last_activity_ms = at_ms;
                self.flush_tables(&tables)?;
            }
        }
        Ok(())
    }

    async fn set_read_mark(
        &self,
        participant: &ParticipantId,
        wave: &str,
        at_ms: u64,
    ) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables
            .read_marks
            .entry(participant.to_string())
            .or_default()
            .insert(wave.to_string(), at_ms);
        self.flush_tables(&tables)
    }

    async fn read_marks(&self, participant: &ParticipantId) -> io::Result<HashMap<String, u64>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables
            .read_marks
            .get(&participant.to_string())
            .cloned()
            .unwrap_or_default())
    }

    async fn put_attachment(&self, meta: &AttachmentMeta) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables.attachments.insert(meta.hash.clone(), meta.clone());
        self.flush_tables(&tables)
    }

    async fn get_attachment(&self, hash: &str) -> io::Result<Option<AttachmentMeta>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.attachments.get(hash).cloned())
    }

    async fn list_attachments(&self, wave: &str) -> io::Result<Vec<AttachmentMeta>> {
        let tables = self.tables.lock().unwrap();
        let mut out: Vec<AttachmentMeta> = tables
            .attachments
            .values()
            .filter(|a| a.wave == wave)
            .cloned()
            .collect();
        out.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
        Ok(out)
    }

    async fn put_peer_key(&self, domain: &str, public_key_hex: &str) -> io::Result<()> {
        let mut tables = self.tables.lock().unwrap();
        tables
            .peer_keys
            .insert(domain.to_string(), public_key_hex.to_string());
        self.flush_tables(&tables)
    }

    async fn get_peer_key(&self, domain: &str) -> io::Result<Option<String>> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.peer_keys.get(domain).cloned())
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

    #[tokio::test]
    async fn update_log_roundtrip_and_snapshot_tail() {
        let (_g, store) = tmp_store();
        let name: WaveletName = "example.org/w+1/conv+root".parse().unwrap();
        assert_eq!(store.append_update(&name, b"u1").await.unwrap(), 1);
        assert_eq!(store.append_update(&name, b"u2").await.unwrap(), 2);
        store.save_snapshot(&name, b"snap-at-2", 2).await.unwrap();
        assert_eq!(store.append_update(&name, b"u3").await.unwrap(), 3);

        let rec = store.load_wavelet(&name).await.unwrap();
        assert_eq!(rec.snapshot.as_deref(), Some(&b"snap-at-2"[..]));
        assert_eq!(rec.tail, vec![b"u3".to_vec()]);
        assert_eq!(rec.total_updates, 3);

        // Playback reads the full log regardless of snapshots (FR-26).
        let all = store.read_all_updates(&name).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0], b"u1");
    }

    #[tokio::test]
    async fn accounts_sessions_waves_marks_attachments() {
        let (_g, store) = tmp_store();
        let ada: ParticipantId = "ada@example.org".parse().unwrap();
        let acct = Account {
            participant: ada.to_string(),
            password_hash: "phc".into(),
            created_ms: 1,
        };
        assert!(store.create_account(&acct).await.unwrap());
        assert!(!store.create_account(&acct).await.unwrap());
        assert!(store.get_account(&ada).await.unwrap().is_some());

        store.put_session("sid1", &ada).await.unwrap();
        assert_eq!(store.get_session("sid1").await.unwrap(), Some(ada.clone()));
        store.delete_session("sid1").await.unwrap();
        assert_eq!(store.get_session("sid1").await.unwrap(), None);

        for (i, wave) in ["example.org/w+a", "example.org/w+b"].iter().enumerate() {
            store
                .put_wave(&WaveMeta {
                    wave: wave.to_string(),
                    title: format!("wave {i}"),
                    participants: vec![ada.to_string()],
                    created_by: ada.to_string(),
                    created_ms: i as u64,
                    last_activity_ms: i as u64,
                    acl_version: 1,
                    translation_enabled: false,
                })
                .await
                .unwrap();
        }
        store.touch_wave("example.org/w+a", 99).await.unwrap();
        let list = store.list_waves_for(&ada).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].wave, "example.org/w+a"); // most recent first

        store
            .set_read_mark(&ada, "example.org/w+a", 50)
            .await
            .unwrap();
        let marks = store.read_marks(&ada).await.unwrap();
        assert_eq!(marks.get("example.org/w+a"), Some(&50));

        let att = AttachmentMeta {
            hash: "abc123".into(),
            wave: "example.org/w+a".into(),
            name: "notes.md".into(),
            mime: "text/markdown".into(),
            size: 42,
            uploader: ada.to_string(),
            created_ms: 7,
        };
        store.put_attachment(&att).await.unwrap();
        assert_eq!(
            store.get_attachment("abc123").await.unwrap().unwrap().name,
            "notes.md"
        );
        assert_eq!(
            store
                .list_attachments("example.org/w+a")
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
