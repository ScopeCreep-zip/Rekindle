//! `end_all_calls` leaves no call behind when the session ends (plan C7.27).

use x25519_dalek::StaticSecret;

use super::mocks::{seed_outgoing_call, MockDeps};
use crate::group_state::{GroupCallState, GroupCallStatus};
use crate::signaling::event::CallSignalEvent;
use crate::signaling::registry::{CallRegistry, GroupCallRegistry};
use crate::state::CallKind;

#[tokio::test]
async fn ending_the_session_ends_every_call() {
    let deps = MockDeps::new();
    seed_outgoing_call(&deps, "dm-1", &"bb".repeat(32), CallKind::Audio);
    seed_outgoing_call(&deps, "dm-2", &"cc".repeat(32), CallKind::Video);
    deps.group_registry.insert(GroupCallState {
        call_id: "group-1".into(),
        initiator_pubkey: "aa".repeat(32),
        kind: 0,
        participants: vec!["aa".repeat(32), "dd".repeat(32)],
        accepted: std::collections::HashSet::new(),
        our_x25519_secret: Some(StaticSecret::random_from_rng(rand::rngs::OsRng)),
        call_key: Some([0xAA; 32]),
        status: GroupCallStatus::Outgoing,
    });

    crate::signaling::end_all_calls(deps.as_ref(), "signed out").await;

    assert!(deps.registry.list_all().is_empty());
    assert!(deps.group_registry.list_ids().is_empty());
    let events = deps.emitted_events();
    let ended: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            CallSignalEvent::CallEnded {
                call_id, reason, ..
            } => {
                assert_eq!(reason, "signed out");
                Some(call_id.as_str())
            }
            CallSignalEvent::GroupCallEnded { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ended.len(), 3, "{ended:?}");
    for id in ["dm-1", "dm-2", "group-1"] {
        assert!(ended.contains(&id), "{id} not ended: {ended:?}");
    }
}

#[tokio::test]
async fn ending_with_no_calls_is_a_no_op() {
    let deps = MockDeps::new();
    crate::signaling::end_all_calls(deps.as_ref(), "signed out").await;
    assert!(deps.emitted_events().is_empty());
}
