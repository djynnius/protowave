//! PostgreSQL backend tests. Gated on PROTOWAVE_TEST_PG (a tokio-postgres
//! connection string, e.g.
//! `host=10.102.109.186 user=protowave password=... dbname=protowave_test`).
//! Skips silently when unset so CI without a database stays green.

use protowave_core::{ParticipantId, WaveletName};
use protowave_server::store::{Account, AttachmentMeta, WaveMeta, WaveStore};
use protowave_server::store_pg::PgStore;

async fn pg() -> Option<PgStore> {
    let url = std::env::var("PROTOWAVE_TEST_PG").ok()?;
    let store = PgStore::connect(&url).await.expect("connect test db");
    Some(store)
}

async fn wipe(store: &PgStore, url: &str) {
    // Fresh tables per run: reconnect with a raw client and truncate.
    let (client, conn) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(conn);
    client
        .batch_execute(
            "TRUNCATE accounts, sessions, waves, wavelet_updates, wavelet_snapshots, read_marks, attachments",
        )
        .await
        .unwrap();
    let _ = store;
}

#[tokio::test]
async fn pg_store_full_contract() {
    let Some(store) = pg().await else {
        eprintln!("PROTOWAVE_TEST_PG unset; skipping PG tests");
        return;
    };
    wipe(&store, &std::env::var("PROTOWAVE_TEST_PG").unwrap()).await;

    // Update log + snapshot + playback.
    let name: WaveletName = "example.org/w+pg/conv+root".parse().unwrap();
    assert_eq!(store.append_update(&name, b"u1").await.unwrap(), 1);
    assert_eq!(store.append_update(&name, b"u2").await.unwrap(), 2);
    store.save_snapshot(&name, b"snap", 2).await.unwrap();
    assert_eq!(store.append_update(&name, b"u3").await.unwrap(), 3);

    let rec = store.load_wavelet(&name).await.unwrap();
    assert_eq!(rec.snapshot.as_deref(), Some(&b"snap"[..]));
    assert_eq!(rec.tail, vec![b"u3".to_vec()]);
    assert_eq!(rec.total_updates, 3);
    assert_eq!(store.read_all_updates(&name).await.unwrap().len(), 3);

    // Accounts + sessions.
    let ada: ParticipantId = "ada@example.org".parse().unwrap();
    let acct = Account {
        participant: ada.to_string(),
        password_hash: "phc".into(),
        created_ms: 1,
        first_name: String::new(),
        last_name: String::new(),
    };
    assert!(store.create_account(&acct).await.unwrap());
    assert!(!store.create_account(&acct).await.unwrap());
    store.put_session("sid", &ada).await.unwrap();
    assert_eq!(store.get_session("sid").await.unwrap(), Some(ada.clone()));
    store.delete_session("sid").await.unwrap();
    assert_eq!(store.get_session("sid").await.unwrap(), None);

    // Waves + participants containment query + read marks.
    store
        .put_wave(&WaveMeta {
            wave: "example.org/w+pg".into(),
            title: "pg wave".into(),
            participants: vec![ada.to_string()],
            created_by: ada.to_string(),
            created_ms: 1,
            last_activity_ms: 1,
            acl_version: 1,
            translation_enabled: false,
            archived: false,
        })
        .await
        .unwrap();
    store.touch_wave("example.org/w+pg", 42).await.unwrap();
    let list = store.list_waves_for(&ada).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].last_activity_ms, 42);

    store
        .set_read_mark(&ada, "example.org/w+pg", 40)
        .await
        .unwrap();
    let marks = store.read_marks(&ada).await.unwrap();
    assert_eq!(marks.get("example.org/w+pg"), Some(&40));

    // Attachments.
    store
        .put_attachment(&AttachmentMeta {
            hash: "h1".into(),
            wave: "example.org/w+pg".into(),
            name: "a.md".into(),
            mime: "text/markdown".into(),
            size: 5,
            uploader: ada.to_string(),
            created_ms: 9,
        })
        .await
        .unwrap();
    assert!(store.get_attachment("h1").await.unwrap().is_some());
    assert_eq!(
        store
            .list_attachments("example.org/w+pg")
            .await
            .unwrap()
            .len(),
        1
    );
}
