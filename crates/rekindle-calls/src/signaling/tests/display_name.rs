//! `friend_display_name` fallback contract.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_protocol::messaging::envelope::MessagePayload;
use x25519_dalek::StaticSecret;

use super::mocks::*;
use crate::error::CallError;
use crate::group_state::{GroupCallState, GroupCallStatus};
use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::event::CallSignalEvent;
use crate::signaling::registry::{CallRegistry, GroupCallRegistry};
use crate::state::CallKind;

// ─── friend_display_name fallback contract ───────────────────────────
//
// The deps trait's `friend_display_name` MUST return the raw value
// (possibly empty) — NOT pre-fallback to short_pubkey. The crate
// handlers detect empty + apply `short_pubkey(initiator_pubkey)` as
// the fallback (which may differ from `peer_pubkey`). If the adapter
// pre-applies a fallback, the crate's "use initiator_pubkey" branch
// is bypassed.

struct EmptyNameDeps(Arc<MockDeps>);

#[async_trait]
impl CallSignalingDeps for EmptyNameDeps {
    fn owner_key(&self) -> Result<String, CallError> {
        self.0.owner_key()
    }
    fn identity_secret(&self) -> Result<[u8; 32], CallError> {
        self.0.identity_secret()
    }
    fn registry(&self) -> Arc<dyn CallRegistry> {
        self.0.registry()
    }
    fn group_registry(&self) -> Arc<dyn GroupCallRegistry> {
        self.0.group_registry()
    }
    fn is_peer_temp_muted(&self, p: &str) -> bool {
        self.0.is_peer_temp_muted(p)
    }
    /// Returns empty — simulates "friend not found in friends map".
    fn friend_display_name(&self, _peer: &str) -> String {
        String::new()
    }
    fn local_video_decode_codecs(&self) -> Vec<String> {
        self.0.local_video_decode_codecs()
    }
    async fn send_to_peer(&self, p: &str, msg: MessagePayload) -> Result<(), CallError> {
        self.0.send_to_peer(p, msg).await
    }
    async fn start_voice_session(
        &self,
        c: &str,
        p: &str,
        k: [u8; 32],
        kind: CallKind,
    ) -> Result<(), CallError> {
        self.0.start_voice_session(c, p, k, kind).await
    }
    async fn shutdown_voice_session(&self) {
        self.0.shutdown_voice_session().await;
    }
    fn voice_active(&self) -> bool {
        self.0.voice_active()
    }
    fn pre_stage_voice_channel(&self) {
        self.0.pre_stage_voice_channel();
    }
    fn persist_missed_call(&self, c: &str, p: &str, k: CallKind, e: u64) {
        self.0.persist_missed_call(c, p, k, e);
    }
    fn surface_window_for_call(&self, c: &str) {
        self.0.surface_window_for_call(c);
    }
    fn emit_event(&self, e: CallSignalEvent) {
        self.0.emit_event(e);
    }
    fn register_background_handle(&self, h: tokio::task::JoinHandle<()>) {
        self.0.register_background_handle(h);
    }
    fn spawn_incoming_call_timeout(&self, c: String, p: String, k: CallKind, e: u64) {
        self.0.spawn_incoming_call_timeout(c, p, k, e);
    }
    fn spawn_dialing_call_timeout(&self, c: String, p: String, k: CallKind, e: u64) {
        self.0.spawn_dialing_call_timeout(c, p, k, e);
    }
}

#[tokio::test]
async fn empty_friend_display_name_falls_back_to_initiator_pubkey() {
    let inner = MockDeps::new();
    let deps = Arc::new(EmptyNameDeps(Arc::clone(&inner)));

    let sender = "bb".repeat(32);
    // Use a DIFFERENT initiator_pubkey from sender to verify the
    // fallback picks initiator_pubkey (the crate's intent).
    let initiator = "ff".repeat(32);
    let initiator_x = peer_x25519_pub_bytes();

    crate::signaling::handlers::handle_incoming_invite(
        deps.as_ref(),
        crate::signaling::handlers::IncomingInvite {
            sender_hex: &sender,
            call_id: "call-fb",
            offer_kind: 0,
            initiator_pubkey: &initiator,
            initiator_x25519_pub: &initiator_x,
            expires_at_ms: 12_345_678,
            video_decode_codecs: &["vp9".to_string()],
        },
    )
    .await;

    // Find the emitted IncomingCall and check its display_name.
    let incoming = inner
        .emitted_events()
        .into_iter()
        .find_map(|e| match e {
            CallSignalEvent::IncomingCall {
                from_display_name, ..
            } => Some(from_display_name),
            _ => None,
        })
        .expect("IncomingCall must be emitted");

    // Expected: short_pubkey(initiator_pubkey) — "ff…" prefix, NOT
    // sender_hex's "bb…" prefix.
    assert!(
        incoming.starts_with("ff"),
        "Empty friend_display_name must fall back to short_pubkey(initiator_pubkey). \
         Got: {incoming:?} — expected to start with 'ff' (initiator), not 'bb' (sender)."
    );
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
            initiator_pubkey: &initiator,
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
