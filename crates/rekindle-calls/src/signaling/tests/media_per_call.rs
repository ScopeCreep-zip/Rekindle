//! Plan CE2 (lead decision 30): call media is owned and torn down per
//! call id. Live CE2: a superseding call rang and was accepted, then the
//! stale call's end shut the one voice session down, and no media flowed.

use super::mocks::{peer_x25519_pub_bytes, seed_call, seed_outgoing_call, MockDeps};
use crate::signaling::event::CallSignalEvent;
use crate::signaling::handlers::{handle_accept_received, handle_end_received};
use crate::signaling::lifecycle::CALL_SUPERSEDED;
use crate::signaling::registry::CallRegistry;
use crate::state::{CallKind, CallStatus};

fn ended(deps: &MockDeps) -> Vec<(String, String)> {
    deps.emitted_events()
        .into_iter()
        .filter_map(|e| match e {
            CallSignalEvent::CallEnded {
                call_id, reason, ..
            } => Some((call_id, reason)),
            _ => None,
        })
        .collect()
}

/// The caller holds the stale call `old` (its media running) and places
/// `new` to the same peer; the peer accepts `new`. The stale call ends
/// here before `new`'s media starts, and the peer's late `CallEnd` for
/// `old` leaves `new`'s media running.
#[tokio::test]
async fn the_stale_calls_end_never_stops_the_new_calls_media() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    seed_call(&deps, "old", &peer, CallStatus::Active);
    deps.media.claim("old");
    seed_outgoing_call(&deps, "new", &peer, CallKind::Audio);

    handle_accept_received(deps.as_ref(), &peer, "new", &peer_x25519_pub_bytes(), &[]).await;
    assert_eq!(
        ended(&deps),
        vec![("old".to_string(), CALL_SUPERSEDED.to_string())]
    );
    assert_eq!(deps.media.owner().as_deref(), Some("new"));

    handle_end_received(deps.as_ref(), &peer, "old", CALL_SUPERSEDED.into()).await;
    assert_eq!(deps.media.owner().as_deref(), Some("new"));
    assert!(deps
        .registry
        .get("new")
        .is_some_and(|c| matches!(c.status, CallStatus::Active)));
}

/// A `CallEnd` for a call still held whose media is not running (the new
/// call took it) removes that call and stops nothing else.
#[tokio::test]
async fn ending_one_call_never_stops_another_calls_media() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    seed_call(&deps, "old", &peer, CallStatus::Active);
    seed_call(&deps, "new", &peer, CallStatus::Active);
    deps.media.claim("new");

    handle_end_received(deps.as_ref(), &peer, "old", "hangup".into()).await;
    assert!(!deps.registry.contains("old"));
    assert_eq!(deps.media.owner().as_deref(), Some("new"));

    crate::signaling::outbound::end_dm_call(deps.as_ref(), "new", None)
        .await
        .expect("held");
    assert_eq!(deps.media.owner(), None);
}

/// A `CallEnd` from a peer that is not the call's is ignored.
#[tokio::test]
async fn a_call_end_from_another_peer_is_ignored() {
    let deps = MockDeps::new();
    seed_call(&deps, "c1", &"bb".repeat(32), CallStatus::Active);
    deps.media.claim("c1");
    handle_end_received(deps.as_ref(), &"cc".repeat(32), "c1", "x".into()).await;
    assert!(deps.registry.contains("c1"));
    assert_eq!(deps.media.owner().as_deref(), Some("c1"));
}
