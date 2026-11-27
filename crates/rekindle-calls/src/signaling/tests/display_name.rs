//! Inbound call display names and related signaling behavior.

use x25519_dalek::StaticSecret;

use super::mocks::*;
use crate::group_state::{GroupCallState, GroupCallStatus};
use crate::signaling::event::CallSignalEvent;
use crate::signaling::registry::GroupCallRegistry;
use crate::state::CallKind;

#[tokio::test]
async fn an_unnamed_caller_is_shown_by_the_sender_fingerprint() {
    let deps = MockDeps::new();
    let sender = "bb".repeat(32);
    let initiator_x = peer_x25519_pub_bytes();

    crate::signaling::handlers::handle_incoming_invite(
        deps.as_ref(),
        crate::signaling::handlers::IncomingInvite {
            sender_hex: &sender,
            call_id: "call-fb",
            offer_kind: 0,
            initiator_x25519_pub: &initiator_x,
            expires_at_ms: 12_345_678,
            video_decode_codecs: &["vp9".to_string()],
        },
    )
    .await;

    let incoming = deps
        .emitted_events()
        .into_iter()
        .find_map(|event| match event {
            CallSignalEvent::IncomingCall {
                from_display_name, ..
            } => Some(from_display_name),
            _ => None,
        })
        .expect("incoming call");
    assert_eq!(incoming, "bbbb bbbb");
}

#[tokio::test]
async fn handle_incoming_invite_arms_incoming_timeout() {
    let deps = MockDeps::new();
    let initiator = "bb".repeat(32);

    let initiator_x = peer_x25519_pub_bytes();
    crate::signaling::handlers::handle_incoming_invite(
        deps.as_ref(),
        crate::signaling::handlers::IncomingInvite {
            sender_hex: &initiator,
            call_id: "call-incoming-1",
            offer_kind: 0, // CallKind::Audio
            initiator_x25519_pub: &initiator_x,
            expires_at_ms: 12_345_678,
            video_decode_codecs: &["vp9".to_string()],
        },
    )
    .await;

    let timeout_arm = deps
        .call_log()
        .into_iter()
        .find(|e| matches!(e, MockEvent::SpawnIncomingTimeout { .. }));
    let Some(MockEvent::SpawnIncomingTimeout {
        call_id,
        peer_pubkey,
        kind,
        expires_at_ms,
    }) = timeout_arm
    else {
        panic!(
            "handle_incoming_invite MUST arm spawn_incoming_call_timeout — \
                otherwise missed-call notifications never fire. Call log: {:?}",
            deps.call_log()
        );
    };
    assert_eq!(call_id, "call-incoming-1");
    assert_eq!(peer_pubkey, "bb".repeat(32));
    assert_eq!(kind, CallKind::Audio);
    assert_eq!(expires_at_ms, 12_345_678);
}

#[tokio::test]
async fn group_accept_from_non_invitee_ignored() {
    let deps = MockDeps::new();
    let initiator = "aa".repeat(32);
    let p1 = "bb".repeat(32);
    let outsider = "ee".repeat(32);

    deps.group_registry.insert(GroupCallState {
        call_id: "group-3".into(),
        group_id: None,
        initiator_pubkey: initiator.clone(),
        kind: 0,
        participants: vec![initiator, p1],
        accepted: std::collections::HashSet::new(),
        our_x25519_secret: Some(StaticSecret::random_from_rng(rand::rngs::OsRng)),
        call_key: Some([0xAA; 32]),
        status: GroupCallStatus::Outgoing,
    });

    crate::signaling::group_handlers::handle_group_accept_received(
        deps.as_ref(),
        &outsider,
        "group-3",
        &outsider,
    );

    let snapshot = deps.group_registry.snapshot("group-3").unwrap();
    assert_eq!(
        snapshot.status,
        GroupCallStatus::Outgoing,
        "Non-invitee accept must NOT promote status"
    );
    assert_eq!(snapshot.accepted_count, 0);
    let emit_count = deps
        .call_log()
        .iter()
        .filter(|e| matches!(e, MockEvent::Emit(_)))
        .count();
    assert_eq!(emit_count, 0, "Non-invitee accept must emit nothing");
}
