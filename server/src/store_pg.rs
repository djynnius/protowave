//! PostgreSQL `WaveStore` backend (PRD §7 storage, Phase 2).
//!
//! Same contract as `FileStore`; schema is migrated on connect. Update logs
//! are append-only rows keyed (wavelet, seq) — NFR-9 holds here too.

use std::collections::HashMap;
use std::io;

use async_trait::async_trait;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::types::Json;
use tokio_postgres::NoTls;

use protowave_core::{ParticipantId, WaveletName};

use crate::store::{Account, AttachmentMeta, WaveMeta, WaveStore, WaveletRecord};

fn db_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("postgres: {e}"))
}

const MIGRATIONS: &str = "
CREATE TABLE IF NOT EXISTS accounts (
    participant TEXT PRIMARY KEY,
    password_hash TEXT NOT NULL,
    created_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    participant TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS waves (
    wave TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    participants JSONB NOT NULL,
    created_by TEXT NOT NULL,
    created_ms BIGINT NOT NULL,
    last_activity_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS wavelet_updates (
    wavelet TEXT NOT NULL,
    seq BIGINT NOT NULL,
    data BYTEA NOT NULL,
    PRIMARY KEY (wavelet, seq)
);
CREATE TABLE IF NOT EXISTS wavelet_snapshots (
    wavelet TEXT PRIMARY KEY,
    covered BIGINT NOT NULL,
    snapshot BYTEA NOT NULL
);
CREATE TABLE IF NOT EXISTS read_marks (
    participant TEXT NOT NULL,
    wave TEXT NOT NULL,
    at_ms BIGINT NOT NULL,
    PRIMARY KEY (participant, wave)
);
CREATE TABLE IF NOT EXISTS attachments (
    hash TEXT PRIMARY KEY,
    wave TEXT NOT NULL,
    name TEXT NOT NULL,
    mime TEXT NOT NULL,
    size BIGINT NOT NULL,
    uploader TEXT NOT NULL,
    created_ms BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS attachments_wave ON attachments (wave);
CREATE INDEX IF NOT EXISTS waves_activity ON waves (last_activity_ms DESC);
ALTER TABLE waves ADD COLUMN IF NOT EXISTS acl_version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE waves ADD COLUMN IF NOT EXISTS translation_enabled BOOLEAN NOT NULL DEFAULT FALSE;
CREATE TABLE IF NOT EXISTS peer_keys (
    domain TEXT PRIMARY KEY,
    public_key TEXT NOT NULL
);
";

pub struct PgStore {
    pool: Pool,
}

impl PgStore {
    /// `url` is a tokio-postgres connection string, e.g.
    /// `host=10.0.0.5 user=protowave password=... dbname=protowave`.
    pub async fn connect(url: &str) -> io::Result<Self> {
        let config: tokio_postgres::Config = url.parse().map_err(db_err)?;
        let mgr = Manager::from_config(
            config,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(mgr).max_size(8).build().map_err(db_err)?;
        let client = pool.get().await.map_err(db_err)?;
        client.batch_execute(MIGRATIONS).await.map_err(db_err)?;
        Ok(Self { pool })
    }

    async fn client(&self) -> io::Result<deadpool_postgres::Object> {
        self.pool.get().await.map_err(db_err)
    }

    fn row_to_wave(row: &tokio_postgres::Row) -> WaveMeta {
        WaveMeta {
            wave: row.get("wave"),
            title: row.get("title"),
            participants: row.get::<_, Json<Vec<String>>>("participants").0,
            created_by: row.get("created_by"),
            created_ms: row.get::<_, i64>("created_ms") as u64,
            last_activity_ms: row.get::<_, i64>("last_activity_ms") as u64,
            acl_version: row.get::<_, i64>("acl_version") as u64,
            translation_enabled: row.get("translation_enabled"),
        }
    }
}

#[async_trait]
impl WaveStore for PgStore {
    async fn append_update(&self, wavelet: &WaveletName, update: &[u8]) -> io::Result<u64> {
        let client = self.client().await?;
        let key = wavelet.to_string();
        // The engine serializes appends per wavelet, but retry on the PK
        // conflict anyway so a race degrades gracefully.
        for _ in 0..5 {
            let row = client
                .query_one(
                    "INSERT INTO wavelet_updates (wavelet, seq, data)
                     VALUES ($1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM wavelet_updates WHERE wavelet = $1), $2)
                     ON CONFLICT DO NOTHING
                     RETURNING seq",
                    &[&key, &update],
                )
                .await;
            match row {
                Ok(row) => return Ok(row.get::<_, i64>(0) as u64),
                Err(_) => continue,
            }
        }
        Err(db_err("append_update: could not allocate sequence"))
    }

    async fn load_wavelet(&self, wavelet: &WaveletName) -> io::Result<WaveletRecord> {
        let client = self.client().await?;
        let key = wavelet.to_string();
        let snap = client
            .query_opt(
                "SELECT covered, snapshot FROM wavelet_snapshots WHERE wavelet = $1",
                &[&key],
            )
            .await
            .map_err(db_err)?;
        let (snapshot, covered) = match snap {
            Some(row) => (
                Some(row.get::<_, Vec<u8>>("snapshot")),
                row.get::<_, i64>("covered"),
            ),
            None => (None, 0),
        };
        let rows = client
            .query(
                "SELECT data FROM wavelet_updates WHERE wavelet = $1 AND seq > $2 ORDER BY seq",
                &[&key, &covered],
            )
            .await
            .map_err(db_err)?;
        let total = client
            .query_one(
                "SELECT COALESCE(MAX(seq), 0) FROM wavelet_updates WHERE wavelet = $1",
                &[&key],
            )
            .await
            .map_err(db_err)?
            .get::<_, i64>(0);
        Ok(WaveletRecord {
            snapshot,
            tail: rows.into_iter().map(|r| r.get("data")).collect(),
            total_updates: total as u64,
        })
    }

    async fn read_all_updates(&self, wavelet: &WaveletName) -> io::Result<Vec<Vec<u8>>> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT data FROM wavelet_updates WHERE wavelet = $1 ORDER BY seq",
                &[&wavelet.to_string()],
            )
            .await
            .map_err(db_err)?;
        Ok(rows.into_iter().map(|r| r.get("data")).collect())
    }

    async fn save_snapshot(
        &self,
        wavelet: &WaveletName,
        snapshot: &[u8],
        covered_updates: u64,
    ) -> io::Result<()> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO wavelet_snapshots (wavelet, covered, snapshot)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (wavelet) DO UPDATE SET covered = $2, snapshot = $3",
                &[&wavelet.to_string(), &(covered_updates as i64), &snapshot],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn create_account(&self, account: &Account) -> io::Result<bool> {
        let client = self.client().await?;
        let n = client
            .execute(
                "INSERT INTO accounts (participant, password_hash, created_ms)
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                &[
                    &account.participant,
                    &account.password_hash,
                    &(account.created_ms as i64),
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn get_account(&self, participant: &ParticipantId) -> io::Result<Option<Account>> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT participant, password_hash, created_ms FROM accounts WHERE participant = $1",
                &[&participant.to_string()],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| Account {
            participant: r.get("participant"),
            password_hash: r.get("password_hash"),
            created_ms: r.get::<_, i64>("created_ms") as u64,
        }))
    }

    async fn put_session(&self, session_id: &str, participant: &ParticipantId) -> io::Result<()> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO sessions (id, participant) VALUES ($1, $2)
                 ON CONFLICT (id) DO UPDATE SET participant = $2",
                &[&session_id, &participant.to_string()],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_session(&self, session_id: &str) -> io::Result<Option<ParticipantId>> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT participant FROM sessions WHERE id = $1",
                &[&session_id],
            )
            .await
            .map_err(db_err)?;
        Ok(row.and_then(|r| r.get::<_, String>(0).parse().ok()))
    }

    async fn delete_session(&self, session_id: &str) -> io::Result<()> {
        let client = self.client().await?;
        client
            .execute("DELETE FROM sessions WHERE id = $1", &[&session_id])
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn put_wave(&self, meta: &WaveMeta) -> io::Result<()> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO waves (wave, title, participants, created_by, created_ms, last_activity_ms, acl_version, translation_enabled)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (wave) DO UPDATE SET
                    title = $2, participants = $3, last_activity_ms = $6, acl_version = $7, translation_enabled = $8",
                &[
                    &meta.wave,
                    &meta.title,
                    &Json(&meta.participants),
                    &meta.created_by,
                    &(meta.created_ms as i64),
                    &(meta.last_activity_ms as i64),
                    &(meta.acl_version as i64),
                    &meta.translation_enabled,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_wave(&self, wave: &str) -> io::Result<Option<WaveMeta>> {
        let client = self.client().await?;
        let row = client
            .query_opt("SELECT * FROM waves WHERE wave = $1", &[&wave])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(Self::row_to_wave))
    }

    async fn list_waves_for(&self, participant: &ParticipantId) -> io::Result<Vec<WaveMeta>> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT * FROM waves WHERE participants @> to_jsonb($1::text)
                 ORDER BY last_activity_ms DESC",
                &[&participant.to_string()],
            )
            .await
            .map_err(db_err)?;
        Ok(rows.iter().map(Self::row_to_wave).collect())
    }

    async fn touch_wave(&self, wave: &str, at_ms: u64) -> io::Result<()> {
        let client = self.client().await?;
        client
            .execute(
                "UPDATE waves SET last_activity_ms = $2 WHERE wave = $1 AND last_activity_ms < $2",
                &[&wave, &(at_ms as i64)],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn set_read_mark(
        &self,
        participant: &ParticipantId,
        wave: &str,
        at_ms: u64,
    ) -> io::Result<()> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO read_marks (participant, wave, at_ms) VALUES ($1, $2, $3)
                 ON CONFLICT (participant, wave) DO UPDATE SET at_ms = $3",
                &[&participant.to_string(), &wave, &(at_ms as i64)],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn read_marks(&self, participant: &ParticipantId) -> io::Result<HashMap<String, u64>> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT wave, at_ms FROM read_marks WHERE participant = $1",
                &[&participant.to_string()],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("wave"), r.get::<_, i64>("at_ms") as u64))
            .collect())
    }

    async fn put_attachment(&self, meta: &AttachmentMeta) -> io::Result<()> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO attachments (hash, wave, name, mime, size, uploader, created_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (hash) DO NOTHING",
                &[
                    &meta.hash,
                    &meta.wave,
                    &meta.name,
                    &meta.mime,
                    &(meta.size as i64),
                    &meta.uploader,
                    &(meta.created_ms as i64),
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_attachment(&self, hash: &str) -> io::Result<Option<AttachmentMeta>> {
        let client = self.client().await?;
        let row = client
            .query_opt("SELECT * FROM attachments WHERE hash = $1", &[&hash])
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| AttachmentMeta {
            hash: r.get("hash"),
            wave: r.get("wave"),
            name: r.get("name"),
            mime: r.get("mime"),
            size: r.get::<_, i64>("size") as u64,
            uploader: r.get("uploader"),
            created_ms: r.get::<_, i64>("created_ms") as u64,
        }))
    }

    async fn put_peer_key(&self, domain: &str, public_key_hex: &str) -> io::Result<()> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO peer_keys (domain, public_key) VALUES ($1, $2)
                 ON CONFLICT (domain) DO NOTHING",
                &[&domain, &public_key_hex],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_peer_key(&self, domain: &str) -> io::Result<Option<String>> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT public_key FROM peer_keys WHERE domain = $1",
                &[&domain],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| r.get(0)))
    }

    async fn list_attachments(&self, wave: &str) -> io::Result<Vec<AttachmentMeta>> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT * FROM attachments WHERE wave = $1 ORDER BY created_ms DESC",
                &[&wave],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| AttachmentMeta {
                hash: r.get("hash"),
                wave: r.get("wave"),
                name: r.get("name"),
                mime: r.get("mime"),
                size: r.get::<_, i64>("size") as u64,
                uploader: r.get("uploader"),
                created_ms: r.get::<_, i64>("created_ms") as u64,
            })
            .collect())
    }
}
