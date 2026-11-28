//! Plan CE3 (lead decision 30): a `CallInvite` is durable until the
//! callee's `CallRinging`, accept or decline, or its expiry. Live CE3: an
//! invite sent once on a dead cached route never arrived, and the caller
//! rang out silently.

use rekindle_codec::capnp_codec::direct::DirectBody;

use super::mocks::{peer_x25519_pub_bytes, seed_outgoing_call, MockDeps};
use crate::signaling::event::CallSignalEvent;
use crate::signaling::handlers::{handle_incoming_invite, handle_ringing_received, IncomingInvite};
use crate::signaling::outbound::{decline_dm_call, end_dm_call, start_dm_call};
use crate::signaling::registry::CallRegistry;
use crate::state::CallKind;

async fn invite(deps: &MockDeps, sender: &str, call_id: &str) {
    let x = peer_x25519_pub_bytes();
    handle_incoming_invite(
        deps,
        IncomingInvite {
            sender_hex: sender,
            call_id,
            offer_kind: 0,
            initiator_x25519_pub: &x,
            expires_at_ms: 9_999_999_999_999,
            video_decode_codecs: &[],
        },
    )
    .await;
}

fn rang(deps: &MockDeps) -> usize {
    deps.emitted_events()
        .iter()
        .filter(|e| matches!(e, CallSignalEvent::IncomingCall { .. }))
        .count()
}

fn ringings_sent(deps: &MockDeps) -> usize {
    deps.with(|s| {
        s.sent
            .iter()
            .filter(|(_, b)| matches!(b, DirectBody::CallRinging(_)))
            .count()
    })
}

/// The invite is committed to the outbox due by its own expiry, never a
/// one-shot send.
#[tokio::test]
async fn an_invite_is_committed_with_its_expiry_as_deadline() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    let call_id = start_dm_call(deps.as_ref(), &peer, CallKind::Audio)
        .await
        .expect("started");
    let expires_at = deps.registry.get(&call_id).expect("held").expires_at_ms;
    assert_eq!(
        deps.with(|s| s.invites.clone()),
        vec![(peer.clone(), call_id.clone(), expires_at)]
    );
    let sent = deps.with(|s| s.sent.clone());
    assert!(
        !sent
            .iter()
            .any(|(_, b)| matches!(b, DirectBody::CallInvite(_))),
        "{sent:?}"
    );
}

/// The callee's `CallRinging` settles the invite row as answered; a
/// cancelled invite's row is discarded.
#[tokio::test]
async fn a_ringing_answer_settles_the_invite_and_a_cancel_discards_it() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    seed_outgoing_call(&deps, "c1", &peer, CallKind::Audio);
    handle_ringing_received(deps.as_ref(), &peer, "c1").await;
    assert_eq!(
        deps.with(|s| s.settled.clone()),
        vec![("c1".to_string(), true)]
    );

    seed_outgoing_call(&deps, "c2", &peer, CallKind::Audio);
    end_dm_call(deps.as_ref(), "c2", None).await.expect("held");
    assert_eq!(
        deps.with(|s| s.settled.clone()),
        vec![("c1".to_string(), true), ("c2".to_string(), false)]
    );
}

/// A re-sent invite while it rings does not ring twice (it re-sends the
/// ringing answer that settles the caller's row), nor after the callee
/// declined it.
#[tokio::test]
async fn a_resent_invite_never_rings_twice() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    invite(&deps, &peer, "c1").await;
    invite(&deps, &peer, "c1").await;
    assert_eq!(rang(&deps), 1);
    assert_eq!(ringings_sent(&deps), 2);

    decline_dm_call(deps.as_ref(), "c1", None)
        .await
        .expect("held");
    invite(&deps, &peer, "c1").await;
    assert_eq!(rang(&deps), 1);
}
