use super::*;
use crate::repo::fixture::{self, OWNER};
use rekindle_types::friend_store::FriendshipState;

/// A store over the real schema with one friend per `(key, state)`.
fn store_with(friends: &[(&str, &str)]) -> SqliteFriendStore {
    let conn = fixture::conn();
    for (i, (key, state)) in friends.iter().enumerate() {
        conn.execute(
            "INSERT INTO friends (owner_key, public_key, display_name, added_at, \
                 dht_record_key, mailbox_dht_key, friendship_state, current_device_id) \
                 VALUES (?1, ?2, 'Alice', 1700000000000, ?3, ?4, ?5, 'device-id-hex')",
            rusqlite::params![
                OWNER,
                key,
                format!("inbox-{i}"),
                format!("mailbox-{i}"),
                state
            ],
        )
        .unwrap();
    }
    let handle = Arc::new(DbHandle::default());
    handle.set(Db::from(rekindle_asql::Connection::from(conn)));
    SqliteFriendStore::new(handle)
}

/// The store holds no database of its own: after a read, clearing the
/// handle finds no other holder, and a later read is an error rather
/// than a closed database (plan C7.18).
#[tokio::test]
async fn the_store_never_holds_the_database_open() {
    let store = store_with(&[("alice", "accepted")]);
    assert!(store
        .lookup_by_pubkey(OWNER, "alice")
        .await
        .unwrap()
        .is_some());
    store.handle.clear().await.unwrap();
    assert!(matches!(
        store.lookup_by_pubkey(OWNER, "alice").await,
        Err(FriendStoreError::Backend(_))
    ));
}

#[tokio::test]
async fn lookup_by_pubkey_round_trip() {
    let store = store_with(&[("alice", "accepted")]);
    let record = store
        .lookup_by_pubkey(OWNER, "alice")
        .await
        .unwrap()
        .expect("alice should be present");
    assert_eq!(record.pubkey_hex, "alice");
    assert_eq!(record.inbox_record_key, "inbox-0");
    assert_eq!(record.mailbox_record_key, "mailbox-0");
    assert_eq!(record.current_device_id.as_deref(), Some("device-id-hex"));
    assert_eq!(record.display_name, "Alice");
    assert_eq!(record.added_at_us, 1_700_000_000_000_000);
    assert_eq!(record.status, FriendshipState::Accepted);
}

#[tokio::test]
async fn lookup_by_pubkey_misses_other_owner() {
    let store = store_with(&[("alice", "accepted")]);
    assert!(store
        .lookup_by_pubkey("you", "alice")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn is_active_friend_distinguishes_states() {
    let store = store_with(&[
        ("alice", "accepted"),
        ("bob", "pending_out"),
        ("carol", "removing"),
    ]);
    assert!(store.is_active_friend(OWNER, "alice").await.unwrap());
    assert!(!store.is_active_friend(OWNER, "bob").await.unwrap());
    assert!(!store.is_active_friend(OWNER, "carol").await.unwrap());
}

#[tokio::test]
async fn lookup_by_inbox_record_key_finds_match() {
    let store = store_with(&[("alice", "accepted")]);
    let got = store
        .lookup_by_inbox_record_key(OWNER, "inbox-0")
        .await
        .unwrap();
    assert_eq!(got.unwrap().pubkey_hex, "alice");
    assert!(store
        .lookup_by_inbox_record_key(OWNER, "nonexistent")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn lookup_batch_returns_only_matches() {
    let store = store_with(&[("alice", "accepted")]);
    let want: Vec<String> = vec!["alice".into(), "ghost".into()];
    let got = store.lookup_batch_by_pubkey(OWNER, &want).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].pubkey_hex, "alice");
    assert!(store
        .lookup_batch_by_pubkey(OWNER, &[])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn iter_active_filters_pending_and_removing() {
    let store = store_with(&[
        ("alice", "accepted"),
        ("bob", "pending_out"),
        ("carol", "removing"),
    ]);
    let active = store.iter_active(OWNER).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].pubkey_hex, "alice");
}
