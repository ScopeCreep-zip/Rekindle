use super::*;

fn record(pubkey: &str, status: FriendshipState) -> FriendRecord {
    FriendRecord {
        pubkey_hex: pubkey.to_string(),
        inbox_record_key: format!("inbox-{pubkey}"),
        mailbox_record_key: format!("mailbox-{pubkey}"),
        current_device_id: None,
        display_name: format!("friend-{pubkey}"),
        added_at_us: 1_700_000_000_000_000,
        status,
    }
}

#[tokio::test]
async fn lookup_by_pubkey_hits_active() {
    let store = MemoryFriendStore::new();
    store.upsert("me", record("alice", FriendshipState::Accepted));

    let got = store.lookup_by_pubkey("me", "alice").await.unwrap();
    assert_eq!(got.unwrap().display_name, "friend-alice");
}

#[tokio::test]
async fn lookup_by_pubkey_misses_other_owner() {
    let store = MemoryFriendStore::new();
    store.upsert("me", record("alice", FriendshipState::Accepted));

    // Different owner_key — should not see "me"'s friend.
    let got = store.lookup_by_pubkey("you", "alice").await.unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn is_active_friend_excludes_pending_and_removing() {
    let store = MemoryFriendStore::new();
    store.upsert("me", record("alice", FriendshipState::Accepted));
    store.upsert("me", record("bob", FriendshipState::PendingOut));
    store.upsert("me", record("carol", FriendshipState::Removing));

    assert!(store.is_active_friend("me", "alice").await.unwrap());
    assert!(!store.is_active_friend("me", "bob").await.unwrap());
    assert!(!store.is_active_friend("me", "carol").await.unwrap());
    assert!(!store.is_active_friend("me", "dave").await.unwrap()); // unknown
}

#[tokio::test]
async fn iter_active_filters_status_and_owner() {
    let store = MemoryFriendStore::new();
    store.upsert("me", record("alice", FriendshipState::Accepted));
    store.upsert("me", record("bob", FriendshipState::PendingOut));
    store.upsert("you", record("eve", FriendshipState::Accepted));

    let active = store.iter_active("me").await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].pubkey_hex, "alice");
}

#[tokio::test]
async fn lookup_by_inbox_record_key_finds_match() {
    let store = MemoryFriendStore::new();
    store.upsert("me", record("alice", FriendshipState::Accepted));

    let got = store
        .lookup_by_inbox_record_key("me", "inbox-alice")
        .await
        .unwrap();
    assert_eq!(got.unwrap().pubkey_hex, "alice");

    let miss = store
        .lookup_by_inbox_record_key("me", "inbox-nobody")
        .await
        .unwrap();
    assert!(miss.is_none());
}

#[tokio::test]
async fn lookup_batch_by_pubkey_returns_only_known() {
    let store = MemoryFriendStore::new();
    store.upsert("me", record("alice", FriendshipState::Accepted));
    store.upsert("me", record("bob", FriendshipState::PendingOut));

    let want: Vec<String> = vec!["alice".into(), "bob".into(), "carol".into()];
    let got = store.lookup_batch_by_pubkey("me", &want).await.unwrap();
    assert_eq!(got.len(), 2);
    let pubkeys: Vec<&str> = got.iter().map(|r| r.pubkey_hex.as_str()).collect();
    assert!(pubkeys.contains(&"alice"));
    assert!(pubkeys.contains(&"bob"));
}

#[test]
fn the_stored_form_round_trips_and_an_unknown_value_is_an_error() {
    for state in [
        FriendshipState::PendingOut,
        FriendshipState::Accepted,
        FriendshipState::Removing,
    ] {
        assert_eq!(state.as_str().parse::<FriendshipState>(), Ok(state));
    }
    assert!("garbage".parse::<FriendshipState>().is_err());
    assert!("active".parse::<FriendshipState>().is_err());
}
