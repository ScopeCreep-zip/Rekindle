use super::*;
use rekindle_ipc::ConnId;
use rekindle_types::presence::SessionStatus;

fn owner(conn: u64) -> IntentOwner {
    IntentOwner {
        conn: Some(ConnId::Bus(conn)),
        identity: "me".into(),
    }
}

fn back() -> IpcRequest {
    IpcRequest::PresenceSet {
        status: SessionStatus::Online,
    }
}

#[test]
fn an_offer_is_taken_once_by_its_owner_while_it_lasts() {
    let cache = UndoCache::default();
    let offer = cache.offer(
        owner(1),
        back(),
        IntentKind::SetStatus,
        1_000,
        &rekindle_utils::OsRandom,
    );
    assert_eq!(offer.expires_at_ms, 1_000 + UNDO_WINDOW_MS);
    assert!(
        cache.take(offer.id, &owner(2), 1_001).is_none(),
        "another connection"
    );
    assert!(cache.take(offer.id, &owner(1), 1_001).is_some());
    assert!(
        cache.take(offer.id, &owner(1), 1_002).is_none(),
        "taken once"
    );

    let late = cache.offer(
        owner(1),
        back(),
        IntentKind::SetStatus,
        1_000,
        &rekindle_utils::OsRandom,
    );
    assert!(cache
        .take(late.id, &owner(1), 1_000 + UNDO_WINDOW_MS)
        .is_none());
}

#[test]
fn a_lock_drops_its_identitys_offers() {
    let cache = UndoCache::default();
    let offer = cache.offer(
        owner(1),
        back(),
        IntentKind::SetStatus,
        0,
        &rekindle_utils::OsRandom,
    );
    cache.clear_matching(|o| o.identity == "me");
    assert!(cache.take(offer.id, &owner(1), 1).is_none());
}
