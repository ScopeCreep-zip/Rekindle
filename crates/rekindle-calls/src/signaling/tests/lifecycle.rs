//! Plan CE1 (lead decision 22): the local end comes first, `CallEnd` goes
//! through the outbox with a deadline, an Active call ends on RFC 7675's
//! 30 s consent timeout, and a peer's new invite supersedes a stale call.

use rekindle_codec::capnp_codec::direct::{CallClosed, DirectBody};

use super::mocks::{peer_x25519_pub_bytes, seed_call, seed_outgoing_call, MockDeps};
use crate::signaling::event::CallSignalEvent;
use crate::signaling::lifecycle::{
    check_consent, ConsentCheck, CALL_END_DEADLINE_MS, CALL_SUPERSEDED, CONNECTION_LOST,
    CONSENT_TIMEOUT_MS,
};
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

fn incoming(deps: &MockDeps) -> Vec<String> {
    deps.emitted_events()
        .into_iter()
        .filter_map(|e| match e {
            CallSignalEvent::IncomingCall { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect()
}

async fn invite(deps: &MockDeps, sender: &str, call_id: &str) {
    let x = peer_x25519_pub_bytes();
    crate::signaling::handlers::handle_incoming_invite(
        deps,
        crate::signaling::handlers::IncomingInvite {
            sender_hex: sender,
            call_id,
            offer_kind: 0,
            initiator_pubkey: sender,
            initiator_x25519_pub: &x,
            expires_at_ms: 9_999_999_999_999,
            video_decode_codecs: &[],
        },
    )
    .await;
}

/// (a) The hang-up's `CallEnd` is committed to the outbox, due two ring
/// windows from now, never a one-shot send whose failure is only logged.
#[tokio::test]
async fn a_hang_up_commits_call_end_to_the_outbox_with_its_deadline() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    seed_call(&deps, "c1", &peer, CallStatus::Active);
    crate::signaling::end_dm_call(deps.as_ref(), "c1", Some("hangup".into()))
        .await
        .expect("held");
    let (committed, sent, now) = deps.with(|s| (s.committed.clone(), s.sent.clone(), s.now_ms));
    assert_eq!(committed.len(), 1, "{committed:?}");
    let (to, body, deadline) = &committed[0];
    assert_eq!(to, &peer);
    assert_eq!(*deadline, now + CALL_END_DEADLINE_MS);
    assert!(
        matches!(body, DirectBody::CallEnd(CallClosed { call_id, reason }) if call_id == "c1" && reason == "hangup")
    );
    assert!(sent.is_empty(), "no one-shot send: {sent:?}");
    assert_eq!(deps.with(|s| s.shutdowns), 1, "voice torn down");
}

/// (d) The local end (and the call window, retired on `CallEnded`) never
/// waits on the network: with a commit that never completes, the call is
/// already gone and `CallEnded` already emitted.
#[tokio::test]
async fn a_hang_up_ends_locally_before_the_network() {
    let deps = MockDeps::new();
    seed_call(&deps, "c1", &"bb".repeat(32), CallStatus::Active);
    deps.with(|s| s.commit_hangs = true);
    let hang = crate::signaling::end_dm_call(deps.as_ref(), "c1", None);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), hang)
            .await
            .is_err(),
        "the mock commit hangs"
    );
    assert_eq!(ended(&deps), vec![("c1".to_string(), String::new())]);
    assert!(!deps.registry.contains("c1"));
    assert_eq!(deps.with(|s| s.shutdowns), 1);
}

/// (b) Live while proof of life is fresh: the next check is due when the
/// 30 s window from the last packet runs out.
#[tokio::test]
async fn consent_holds_while_media_arrives() {
    let deps = MockDeps::new();
    seed_call(&deps, "c1", &"bb".repeat(32), CallStatus::Active);
    let now = deps.with(|s| {
        s.last_inbound_ms = Some(s.now_ms - 10_000);
        s.now_ms
    });
    let check = check_consent(deps.as_ref(), "c1", now - 60_000).await;
    assert_eq!(
        check,
        ConsentCheck::Live {
            recheck_in_ms: CONSENT_TIMEOUT_MS - 10_000
        }
    );
    assert!(ended(&deps).is_empty());
}

/// (b) 30 s with no media or receiver report ends the call here with
/// `connection-lost`: voice down, `CallEnded`, `CallEnd` committed.
#[tokio::test]
async fn consent_expires_after_thirty_silent_seconds() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    seed_call(&deps, "c1", &peer, CallStatus::Active);
    let now = deps.with(|s| {
        s.last_inbound_ms = Some(s.now_ms - CONSENT_TIMEOUT_MS);
        s.now_ms
    });
    let check = check_consent(deps.as_ref(), "c1", now - 120_000).await;
    assert_eq!(check, ConsentCheck::Expired);
    assert_eq!(
        ended(&deps),
        vec![("c1".to_string(), CONNECTION_LOST.to_string())]
    );
    assert!(!deps.registry.contains("c1"));
    assert_eq!(deps.with(|s| (s.shutdowns, s.committed.len())), (1, 1));
}

/// (b) Before the first packet the window runs from the call's start, and
/// a call no longer Active is not watched.
#[tokio::test]
async fn consent_runs_from_the_start_and_stops_when_the_call_is_gone() {
    let deps = MockDeps::new();
    seed_call(&deps, "c1", &"bb".repeat(32), CallStatus::Active);
    let now = deps.with(|s| s.now_ms);
    let check = check_consent(deps.as_ref(), "c1", now - 5_000).await;
    assert_eq!(
        check,
        ConsentCheck::Live {
            recheck_in_ms: CONSENT_TIMEOUT_MS - 5_000
        }
    );
    deps.registry.remove("c1");
    assert_eq!(
        check_consent(deps.as_ref(), "c1", 0).await,
        ConsentCheck::Gone
    );
}

/// (b) Both sides watch: the caller once the peer's accept brings voice up.
#[tokio::test]
async fn an_accepted_call_starts_its_consent_watch() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    seed_outgoing_call(&deps, "c1", &peer, CallKind::Audio);
    crate::signaling::handlers::handle_accept_received(
        deps.as_ref(),
        &peer,
        "c1",
        &peer_x25519_pub_bytes(),
        &[],
    )
    .await;
    let (watches, now) = deps.with(|s| (s.consent_watches.clone(), s.now_ms));
    assert_eq!(watches, vec![("c1".to_string(), now)]);
}

/// (c) A new invite from the peer we are in a call with ends the stale
/// call here and rings.
#[tokio::test]
async fn a_new_invite_from_our_call_peer_supersedes_the_stale_call() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    seed_call(&deps, "old", &peer, CallStatus::Active);
    invite(&deps, &peer, "new").await;
    assert_eq!(
        ended(&deps),
        vec![("old".to_string(), CALL_SUPERSEDED.to_string())]
    );
    assert!(!deps.registry.contains("old"));
    assert_eq!(incoming(&deps), vec!["new".to_string()]);
    assert_eq!(deps.with(|s| s.shutdowns), 1);
}

/// (c) An invite from another peer while we are in a call is answered busy
/// and never rings.
#[tokio::test]
async fn an_invite_from_another_peer_while_in_a_call_is_answered_busy() {
    let deps = MockDeps::new();
    seed_call(&deps, "ours", &"bb".repeat(32), CallStatus::Active);
    let other = "cc".repeat(32);
    invite(&deps, &other, "theirs").await;
    assert!(incoming(&deps).is_empty());
    assert!(!deps.registry.contains("theirs"));
    assert!(deps.registry.contains("ours"));
    let sent = deps.with(|s| s.sent.clone());
    assert!(
        sent.iter().any(|(to, b)| to == &other
            && matches!(b, DirectBody::CallDecline(CallClosed { call_id, reason }) if call_id == "theirs" && reason == crate::signaling::handlers::BUSY)),
        "{sent:?}"
    );
}

/// (c) A repeated invite for a call we hold does not ring again.
#[tokio::test]
async fn a_repeated_invite_does_not_ring_twice() {
    let deps = MockDeps::new();
    let peer = "bb".repeat(32);
    invite(&deps, &peer, "c1").await;
    invite(&deps, &peer, "c1").await;
    assert_eq!(incoming(&deps), vec!["c1".to_string()]);
}
