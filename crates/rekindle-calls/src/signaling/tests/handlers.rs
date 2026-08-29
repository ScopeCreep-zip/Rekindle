//! Handler-level regressions: pre-stage ordering, CallConnected
//! kind propagation, video-codec storage, group accept.

use x25519_dalek::StaticSecret;

use super::mocks::*;
use crate::group_state::{GroupCallState, GroupCallStatus};
use crate::signaling::event::CallSignalEvent;
use crate::signaling::registry::{CallRegistry, GroupCallRegistry};
use crate::state::CallKind;

#[tokio::test]
async fn w14_1_pre_stage_runs_before_start_voice_session() {
    let deps = MockDeps::new();
    seed_outgoing_call(&deps, "call-1", &"bb".repeat(32), CallKind::Audio);

    let acceptor_pub = peer_x25519_pub_bytes();
    crate::signaling::handlers::handle_accept_received(
        deps.as_ref(),
        &"bb".repeat(32),
        "call-1",
        &acceptor_pub,
        &["vp9".to_string()],
    )
    .await;

    let log = deps.call_log();
    let pre_idx = log
        .iter()
        .position(|e| matches!(e, MockEvent::PreStage))
        .expect("pre_stage_voice_channel must be called");
    let start_idx = log
        .iter()
        .position(|e| matches!(e, MockEvent::StartVoiceSession { .. }))
        .expect("start_voice_session must be called");
    assert!(
        pre_idx < start_idx,
        "W14.1 violated: pre_stage_voice_channel (idx {pre_idx}) must precede \
         start_voice_session (idx {start_idx}). Order: {log:?}"
    );
}

// ─── Phase 5: peer video decode codecs stored from invite + accept ────

#[tokio::test]
async fn invite_stores_peer_video_decode_codecs() {
    let deps = MockDeps::new();
    let sender = "bb".repeat(32);
    let initiator_x = peer_x25519_pub_bytes();
    crate::signaling::handlers::handle_incoming_invite(
        deps.as_ref(),
        crate::signaling::handlers::IncomingInvite {
            sender_hex: &sender,
            call_id: "call-codecs-i",
            offer_kind: 1,
            initiator_pubkey: &sender,
            initiator_x25519_pub: &initiator_x,
            expires_at_ms: 9_999_999_999_999,
            video_decode_codecs: &["vp8".to_string(), "h264".to_string()],
        },
    )
    .await;
    let call = deps
        .registry
        .get("call-codecs-i")
        .expect("invite must insert CallState");
    assert_eq!(call.peer_video_decode_codecs, vec!["vp8", "h264"]);
}

#[tokio::test]
async fn accept_stores_peer_video_decode_codecs() {
    let deps = MockDeps::new();
    seed_outgoing_call(&deps, "call-codecs-a", &"bb".repeat(32), CallKind::Video);
    let acceptor_pub = peer_x25519_pub_bytes();
    crate::signaling::handlers::handle_accept_received(
        deps.as_ref(),
        &"bb".repeat(32),
        "call-codecs-a",
        &acceptor_pub,
        &["h264".to_string(), "vp9".to_string()],
    )
    .await;
    let call = deps
        .registry
        .get("call-codecs-a")
        .expect("accepted call stays registered");
    assert_eq!(
        call.peer_video_decode_codecs,
        vec!["h264", "vp9"],
        "accept must overwrite the empty seed with the peer's list"
    );
}

// ─── W14.2 regression: CallConnected carries the kind ────────────────

#[tokio::test]
async fn w14_2_call_connected_carries_kind_audio() {
    let deps = MockDeps::new();
    seed_outgoing_call(&deps, "call-a", &"bb".repeat(32), CallKind::Audio);
    let acceptor_pub = peer_x25519_pub_bytes();

    crate::signaling::handlers::handle_accept_received(
        deps.as_ref(),
        &"bb".repeat(32),
        "call-a",
        &acceptor_pub,
        &["vp9".to_string()],
    )
    .await;

    let kind = deps
        .emitted_events()
        .into_iter()
        .find_map(|e| match e {
            CallSignalEvent::CallConnected { kind, .. } => Some(kind),
            _ => None,
        })
        .expect("CallConnected must be emitted");
    assert_eq!(kind, CallKind::Audio);
}

#[tokio::test]
async fn w14_2_call_connected_carries_kind_video() {
    let deps = MockDeps::new();
    seed_outgoing_call(&deps, "call-v", &"bb".repeat(32), CallKind::Video);
    let acceptor_pub = peer_x25519_pub_bytes();

    crate::signaling::handlers::handle_accept_received(
        deps.as_ref(),
        &"bb".repeat(32),
        "call-v",
        &acceptor_pub,
        &["vp9".to_string()],
    )
    .await;

    let kind = deps
        .emitted_events()
        .into_iter()
        .find_map(|e| match e {
            CallSignalEvent::CallConnected { kind, .. } => Some(kind),
            _ => None,
        })
        .expect("CallConnected must be emitted");
    assert_eq!(kind, CallKind::Video);
}

// ─── Group accept regression: status MUST become Active ──────────────

#[tokio::test]
async fn group_accept_first_acceptor_transitions_to_active() {
    let deps = MockDeps::new();
    let initiator = "aa".repeat(32);
    let participant1 = "bb".repeat(32);
    let participant2 = "cc".repeat(32);

    // Seed outgoing group call — initiator dialed, waiting for accepts.
    deps.group_registry.insert(GroupCallState {
        call_id: "group-1".into(),
        initiator_pubkey: initiator.clone(),
        kind: 0,
        participants: vec![initiator, participant1.clone(), participant2.clone()],
        accepted: std::collections::HashSet::new(),
        our_x25519_secret: Some(StaticSecret::random_from_rng(rand::rngs::OsRng)),
        call_key: Some([0xAA; 32]),
        status: GroupCallStatus::Outgoing,
    });

    crate::signaling::group_handlers::handle_group_accept_received(
        deps.as_ref(),
        &participant1,
        "group-1",
        &participant1,
    );

    let snapshot = deps
        .group_registry
        .snapshot("group-1")
        .expect("group call still in registry");
    assert_eq!(
        snapshot.status,
        GroupCallStatus::Active,
        "First-accept must transition status Outgoing → Active. Got: {:?}",
        snapshot.status
    );
    assert_eq!(snapshot.accepted_count, 1, "accept must be recorded");

    // Verify both events emitted in the right order: Connected (first),
    // ParticipantJoined (always).
    let emit_order: Vec<_> = deps
        .call_log()
        .into_iter()
        .filter_map(|e| match e {
            MockEvent::Emit(label) => Some(label),
            _ => None,
        })
        .collect();
    assert_eq!(
        emit_order,
        vec![
            "GroupCallConnected".to_string(),
            "GroupCallParticipantJoined".to_string()
        ],
        "First-accept must emit Connected before ParticipantJoined"
    );
}

#[tokio::test]
async fn group_accept_second_acceptor_does_not_re_emit_connected() {
    let deps = MockDeps::new();
    let initiator = "aa".repeat(32);
    let p1 = "bb".repeat(32);
    let p2 = "cc".repeat(32);

    // Pre-seed: first acceptor already in the set, status already Active.
    let mut accepted = std::collections::HashSet::new();
    accepted.insert(p1.clone());
    deps.group_registry.insert(GroupCallState {
        call_id: "group-2".into(),
        initiator_pubkey: initiator.clone(),
        kind: 0,
        participants: vec![initiator, p1, p2.clone()],
        accepted,
        our_x25519_secret: Some(StaticSecret::random_from_rng(rand::rngs::OsRng)),
        call_key: Some([0xAA; 32]),
        status: GroupCallStatus::Active,
    });

    crate::signaling::group_handlers::handle_group_accept_received(
        deps.as_ref(),
        &p2,
        "group-2",
        &p2,
    );

    let emit_order: Vec<_> = deps
        .call_log()
        .into_iter()
        .filter_map(|e| match e {
            MockEvent::Emit(label) => Some(label),
            _ => None,
        })
        .collect();
    assert_eq!(
        emit_order,
        vec!["GroupCallParticipantJoined".to_string()],
        "Second accept emits only ParticipantJoined (not GroupCallConnected again)"
    );
}

// ─── Incoming-call timeout regression: missed-call timer wired ────────
