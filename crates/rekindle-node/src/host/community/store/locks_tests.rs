use rekindle_records::lease::LeaseId;

use crate::host::community::state::CommunityState;
use crate::host::community::store::{new_map, CommunityStore};

fn store_with(community: &str) -> CommunityStore {
    let store = CommunityStore::new(new_map());
    store.map().write().insert(
        community.to_owned(),
        CommunityState {
            id: community.to_owned(),
            ..CommunityState::default()
        },
    );
    store
}

/// The rebuilds that finish while a follow-up runs cost it one more
/// turn; it stops once a turn finds nothing new.
#[test]
fn follow_ups_coalesce_into_one_task_and_one_more_turn() {
    let store = store_with("c");
    assert!(
        store.mark_follow_up("c"),
        "the first rebuild starts the task"
    );
    assert!(!store.mark_follow_up("c"), "a second rebuild only marks it");
    assert!(!store.mark_follow_up("c"));
    assert!(store.take_follow_up("c"), "one turn for both");
    assert!(!store.take_follow_up("c"), "then the task stops");
    assert!(store.mark_follow_up("c"), "and the next rebuild starts one");
}

/// Leave takes the watch ring's leases with the community's own, so a
/// ring move that finished after `release_watch_ring` leaks none.
#[test]
fn take_leases_takes_the_watch_ring() {
    let store = store_with("c");
    store
        .map()
        .write()
        .get_mut("c")
        .expect("community")
        .watch_ring
        .insert("record".to_owned(), LeaseId(7));
    let (_, ring) = store.take_leases("c");
    assert_eq!(ring, vec![LeaseId(7)]);
    assert!(store.map().read()["c"].watch_ring.is_empty());
}

/// Leave drops the community's locks and follow-up state with it.
#[test]
fn remove_prunes_the_per_community_state() {
    let store = store_with("c");
    assert!(store.mark_follow_up("c"));
    let before = store.frontier_write_lock("c");
    let verdicts = store.op_verify_cache("c");
    store.remove("c");
    assert!(!std::sync::Arc::ptr_eq(
        &verdicts,
        &store.op_verify_cache("c")
    ));
    assert!(!std::sync::Arc::ptr_eq(
        &before,
        &store.frontier_write_lock("c")
    ));
    assert!(store.mark_follow_up("c"), "no follow-up survives the leave");
}

/// A teardown forgets the follow-up and card refresh a stopped task left
/// marked running, so the next session starts them again.
#[test]
fn reset_session_state_forgets_the_running_markers() {
    let store = store_with("c");
    assert!(store.mark_follow_up("c"));
    assert!(store.mark_card_refresh("c"));
    store.reset_session_state();
    assert!(
        store.mark_follow_up("c"),
        "the next session's follow-up starts"
    );
    assert!(store.mark_card_refresh("c"), "and its card refresh");
}
