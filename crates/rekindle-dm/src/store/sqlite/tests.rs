use super::*;

async fn in_memory_store_with_schema() -> SqliteDmStore {
    let conn = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
    conn.call(|c| -> rusqlite::Result<()> {
        c.execute_batch(
            "CREATE TABLE dms (
                owner_key TEXT NOT NULL,
                record_key TEXT NOT NULL,
                is_group INTEGER NOT NULL,
                initiator_public_key TEXT NOT NULL,
                initiator_pseudonym TEXT NOT NULL,
                my_subkey INTEGER NOT NULL,
                participants_json TEXT NOT NULL,
                slot_seed_hex TEXT NOT NULL,
                wrapped_mek_blob BLOB,
                mek_generation INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                last_message_at INTEGER,
                PRIMARY KEY (owner_key, record_key)
            );
            CREATE TABLE dm_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                owner_key TEXT NOT NULL,
                record_key TEXT NOT NULL,
                sender_pseudonym TEXT NOT NULL,
                body TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                sequence INTEGER NOT NULL,
                mek_generation INTEGER NOT NULL
            );",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    SqliteDmStore::new(conn)
}

fn sample_invite() -> DmInvitePending {
    DmInvitePending {
        record_key: "rec123".into(),
        is_group: false,
        initiator_public_key: "pk_initiator".into(),
        initiator_pseudonym: "alice".into(),
        my_subkey: 1,
        participants: vec![],
        mek_generation: 0,
        // 32-byte slot seed as 64 hex chars; required by get_session_meta
        // and the production slot-keypair derivation path.
        slot_seed_hex: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into(),
        wrapped_mek_blob: None,
        created_at: 100,
    }
}

#[tokio::test]
async fn empty_owner_key_rejected_on_persist() {
    let store = in_memory_store_with_schema().await;
    let err = store
        .persist_invite_pending("", sample_invite())
        .await
        .unwrap_err();
    assert!(matches!(err, DmError::InvalidInput(_)));
}

#[tokio::test]
async fn persist_then_list_returns_inserted_row() {
    let store = in_memory_store_with_schema().await;
    store
        .persist_invite_pending("owner1", sample_invite())
        .await
        .unwrap();
    let list = store.list_conversations("owner1").await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].record_key, "rec123");
    assert_eq!(list[0].initiator_pseudonym, "alice");
    assert!(!list[0].is_group);
}

#[tokio::test]
async fn list_scopes_by_owner_key() {
    let store = in_memory_store_with_schema().await;
    store
        .persist_invite_pending("ownerA", sample_invite())
        .await
        .unwrap();
    let mut bob_invite = sample_invite();
    bob_invite.record_key = "rec456".into();
    store
        .persist_invite_pending("ownerB", bob_invite)
        .await
        .unwrap();
    assert_eq!(store.list_conversations("ownerA").await.unwrap().len(), 1);
    assert_eq!(store.list_conversations("ownerB").await.unwrap().len(), 1);
    assert_eq!(store.list_conversations("ownerC").await.unwrap().len(), 0);
}

#[tokio::test]
async fn persist_is_idempotent_on_conflict() {
    let store = in_memory_store_with_schema().await;
    store
        .persist_invite_pending("owner1", sample_invite())
        .await
        .unwrap();
    // Second insert with same (owner_key, record_key) is no-op.
    store
        .persist_invite_pending("owner1", sample_invite())
        .await
        .unwrap();
    assert_eq!(store.list_conversations("owner1").await.unwrap().len(), 1);
}

#[tokio::test]
async fn decline_removes_row() {
    let store = in_memory_store_with_schema().await;
    store
        .persist_invite_pending("owner1", sample_invite())
        .await
        .unwrap();
    store.decline_invite("owner1", "rec123").await.unwrap();
    assert_eq!(store.list_conversations("owner1").await.unwrap().len(), 0);
}

#[tokio::test]
async fn load_messages_returns_oldest_first() {
    let store = in_memory_store_with_schema().await;
    store
        .persist_invite_pending("owner1", sample_invite())
        .await
        .unwrap();
    store
        .conn
        .call(|c| -> rusqlite::Result<()> {
            c.execute(
                "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params!["owner1", "rec123", "alice", "hello", 200_i64, 1_i64, 0_i64],
            )?;
            c.execute(
                "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params!["owner1", "rec123", "bob", "hi back", 300_i64, 2_i64, 0_i64],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let msgs = store.load_messages("owner1", "rec123", 10).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].body, "hello");
    assert_eq!(msgs[1].body, "hi back");
}

#[tokio::test]
async fn get_session_meta_returns_persisted_fields() {
    let store = in_memory_store_with_schema().await;
    store
        .persist_invite_pending("owner1", sample_invite())
        .await
        .unwrap();
    let meta = store
        .get_session_meta("owner1", "rec123")
        .await
        .unwrap()
        .expect("session meta should exist");
    assert_eq!(meta.my_subkey, 1);
    assert_eq!(meta.initiator_pseudonym, "alice");
    assert_eq!(meta.initiator_public_key, "pk_initiator");
    assert!(!meta.is_group);
    assert_eq!(meta.slot_seed.len(), 32);
}

#[tokio::test]
async fn get_session_meta_returns_none_for_missing_row() {
    let store = in_memory_store_with_schema().await;
    let meta = store
        .get_session_meta("owner1", "doesnt-exist")
        .await
        .unwrap();
    assert!(meta.is_none());
}

#[tokio::test]
async fn get_session_meta_rejects_invalid_slot_seed_hex() {
    let store = in_memory_store_with_schema().await;
    store
        .conn
        .call(|c| -> rusqlite::Result<()> {
            c.execute(
                "INSERT INTO dms (owner_key, record_key, is_group, initiator_public_key,
                    initiator_pseudonym, my_subkey, participants_json, slot_seed_hex,
                    wrapped_mek_blob, mek_generation, created_at, last_message_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, NULL)",
                rusqlite::params![
                    "owner1", "badrec", 0_i64, "pk", "alice", 1_i64, "[]", "tooshort", 0_i64,
                    100_i64,
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let err = store
        .get_session_meta("owner1", "badrec")
        .await
        .unwrap_err();
    assert!(matches!(err, DmError::InvalidInput(_)));
}

#[tokio::test]
async fn next_sequence_starts_at_one_for_empty_history() {
    let store = in_memory_store_with_schema().await;
    let seq = store
        .next_sequence_for_sender("owner1", "rec123", "alice")
        .await
        .unwrap();
    assert_eq!(seq, 1);
}

#[tokio::test]
async fn next_sequence_increments_per_sender() {
    let store = in_memory_store_with_schema().await;
    store
        .conn
        .call(|c| -> rusqlite::Result<()> {
            c.execute(
                "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params!["owner1", "rec123", "alice", "m1", 100_i64, 1_i64, 0_i64],
            )?;
            c.execute(
                "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params!["owner1", "rec123", "alice", "m2", 200_i64, 5_i64, 0_i64],
            )?;
            c.execute(
                "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params!["owner1", "rec123", "bob", "m1", 150_i64, 1_i64, 0_i64],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let alice = store
        .next_sequence_for_sender("owner1", "rec123", "alice")
        .await
        .unwrap();
    let bob = store
        .next_sequence_for_sender("owner1", "rec123", "bob")
        .await
        .unwrap();
    assert_eq!(alice, 6, "next after max-seen alice seq 5");
    assert_eq!(bob, 2, "next after max-seen bob seq 1");
}

#[tokio::test]
async fn persist_message_inserts_and_updates_last_message_at() {
    let store = in_memory_store_with_schema().await;
    store
        .persist_invite_pending("owner1", sample_invite())
        .await
        .unwrap();
    let insert = DmMessageInsert {
        record_key: "rec123".into(),
        sender_pseudonym: "alice".into(),
        body: "hi".into(),
        timestamp_secs: 555,
        sequence: 1,
        mek_generation: 0,
    };
    store.persist_message("owner1", insert).await.unwrap();
    let msgs = store.load_messages("owner1", "rec123", 10).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].body, "hi");
    let convos = store.list_conversations("owner1").await.unwrap();
    assert_eq!(convos[0].last_message_at, Some(555));
}

#[tokio::test]
async fn oldest_recent_message_ts_returns_min_within_lookback() {
    let store = in_memory_store_with_schema().await;
    store
        .conn
        .call(|c| -> rusqlite::Result<()> {
            for seq in 1..=5_i64 {
                c.execute(
                    "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        "owner1",
                        "rec123",
                        "alice",
                        format!("m{seq}"),
                        seq * 100,
                        seq,
                        0_i64,
                    ],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let ts = store
        .oldest_recent_message_ts("owner1", "rec123", 3)
        .await
        .unwrap();
    assert_eq!(ts, Some(300));
    let ts_all = store
        .oldest_recent_message_ts("owner1", "rec123", 100)
        .await
        .unwrap();
    assert_eq!(ts_all, Some(100));
}

#[tokio::test]
async fn update_mek_generation_persists() {
    let store = in_memory_store_with_schema().await;
    store
        .persist_invite_pending("owner1", sample_invite())
        .await
        .unwrap();
    store
        .update_mek_generation("owner1", "rec123", 42)
        .await
        .unwrap();
    let convos = store.list_conversations("owner1").await.unwrap();
    assert_eq!(convos[0].mek_generation, 42);
}

#[tokio::test]
async fn load_invite_meta_returns_full_fields() {
    let store = in_memory_store_with_schema().await;
    let mut invite = sample_invite();
    invite.is_group = true;
    invite.wrapped_mek_blob = Some(vec![1, 2, 3, 4]);
    invite.mek_generation = 7;
    invite.participants = vec![GroupDmParticipant {
        pseudonym: "bob".into(),
        subkey: 0,
        public_key: "pk_bob".into(),
    }];
    store
        .persist_invite_pending("owner1", invite)
        .await
        .unwrap();
    let meta = store
        .load_invite_meta("owner1", "rec123")
        .await
        .unwrap()
        .expect("invite meta should exist");
    assert!(meta.is_group);
    assert_eq!(meta.wrapped_mek_blob.as_deref(), Some(&[1u8, 2, 3, 4][..]));
    assert_eq!(meta.mek_generation, 7);
    assert_eq!(meta.my_subkey, 1);
    assert_eq!(meta.participants.len(), 1);
    assert_eq!(meta.participants[0].pseudonym, "bob");
}

#[tokio::test]
async fn load_invite_meta_returns_none_for_missing_row() {
    let store = in_memory_store_with_schema().await;
    let meta = store.load_invite_meta("owner1", "missing").await.unwrap();
    assert!(meta.is_none());
}
