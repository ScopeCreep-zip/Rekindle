//! Phase 14.k — voice mute / deafen handlers + soundboard.
//!
//! Implements the simple per-target state updates: mute/deafen mirror
//! the gossip payload onto our local voice engine if the target is
//! us. SoundboardPlay validates the actor (§9.3 reader-validates) +
//! USE_SOUNDBOARD permission (§10.9) before fanning out to the UI.

use crate::signaling::deps::{perms, CommunityVoiceEvent, VoiceSignalingDeps};

pub(super) fn handle_voice_mute(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    target_pseudonym: &str,
    muted: bool,
) {
    let my_pseudonym = deps.my_pseudonym(community_id);
    if my_pseudonym.as_deref() == Some(target_pseudonym) {
        deps.set_voice_engine_muted(muted);
    }
    deps.emit_event(CommunityVoiceEvent::UserMuted {
        target_pseudonym: target_pseudonym.to_string(),
        muted,
    });
}

pub(super) fn handle_voice_deafen(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    target_pseudonym: &str,
    deafened: bool,
) {
    let my_pseudonym = deps.my_pseudonym(community_id);
    if my_pseudonym.as_deref() == Some(target_pseudonym) {
        deps.set_voice_engine_deafened(deafened);
    }
}

/// §9.3 reader-validates + §10.9 — drop if the gossip-verified sender
/// doesn't match the claimed actor, or if the sender lacks
/// `USE_SOUNDBOARD`. Otherwise any community member could blast
/// audio claiming to be anyone.
pub(super) fn handle_soundboard_play(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
    expression_id: String,
    actor_pseudonym: String,
) {
    if !actor_pseudonym.eq_ignore_ascii_case(sender_pseudonym) {
        tracing::debug!(
            community = %community_id,
            sender = %sender_pseudonym,
            actor = %actor_pseudonym,
            "dropping SoundboardPlay: actor_pseudonym mismatch"
        );
        return;
    }
    if !deps.sender_has_perm(community_id, sender_pseudonym, perms::USE_SOUNDBOARD) {
        tracing::debug!(
            community = %community_id,
            sender = %sender_pseudonym,
            "dropping SoundboardPlay: sender lacks USE_SOUNDBOARD"
        );
        return;
    }
    deps.emit_event(CommunityVoiceEvent::SoundboardPlay {
        community_id: community_id.to_string(),
        channel_id,
        expression_id,
        actor_pseudonym,
    });
}
