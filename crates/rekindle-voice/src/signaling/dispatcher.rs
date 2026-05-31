//! Phase 14.k — voice signaling dispatcher.
//!
//! Entry point for community gossip `ControlPayload` variants. Routes
//! each variant to its dedicated handler module. Stays small per the
//! plan's per-file ≤500 LoC cap (architecture rules §I1 + per-file
//! responsibility — one short noun phrase per filename).

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::signaling::deps::{CommunityVoiceEvent, VoiceSignalingDeps};
use crate::signaling::{mute, presence, stage};

/// Main dispatcher for voice-related `ControlPayload` variants.
pub async fn handle_voice_signaling(
    deps: Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: ControlPayload,
) {
    match payload {
        ControlPayload::VoiceJoin {
            channel_id,
            route_blob,
        } => {
            presence::handle_voice_join(
                &deps,
                community_id,
                sender_pseudonym,
                channel_id,
                route_blob,
            );
        }
        ControlPayload::VoiceLeave { channel_id } => {
            presence::handle_voice_leave(&deps, community_id, sender_pseudonym, channel_id);
        }
        ControlPayload::VoiceModeSwitch {
            channel_id,
            mode,
            host_pseudonym,
        } => {
            deps.emit_event(CommunityVoiceEvent::VoiceModeSwitch {
                community_id: community_id.to_string(),
                channel_id,
                mode,
                host_pseudonym,
            });
        }
        ControlPayload::StageUpdate {
            channel_id,
            topic,
            speakers,
            moderator_pseudonym,
            lamport: _,
        } => {
            stage::handle_stage_update(
                &deps,
                community_id,
                channel_id,
                topic,
                speakers,
                moderator_pseudonym,
            );
        }
        ControlPayload::SpeakRequest {
            channel_id,
            requester_pseudonym,
            lamport: _,
        } => {
            stage::handle_speak_request(
                deps.as_ref(),
                community_id,
                channel_id,
                requester_pseudonym,
            );
        }
        ControlPayload::SpeakResponse {
            channel_id,
            requester_pseudonym,
            granted,
            moderator_pseudonym,
            lamport: _,
        } => {
            stage::handle_speak_response(
                &deps,
                community_id,
                channel_id,
                requester_pseudonym,
                granted,
                moderator_pseudonym,
            );
        }
        ControlPayload::VoiceMute {
            channel_id: _,
            target_pseudonym,
            muted,
        } => {
            mute::handle_voice_mute(deps.as_ref(), community_id, &target_pseudonym, muted);
        }
        ControlPayload::VoiceDeafen {
            channel_id: _,
            target_pseudonym,
            deafened,
        } => {
            mute::handle_voice_deafen(deps.as_ref(), community_id, &target_pseudonym, deafened);
        }
        ControlPayload::VoiceRoster {
            channel_id: _,
            participants,
        } => {
            presence::handle_voice_roster(&deps, participants);
        }
        ControlPayload::SoundboardPlay {
            channel_id,
            expression_id,
            actor_pseudonym,
        } => {
            mute::handle_soundboard_play(
                deps.as_ref(),
                community_id,
                sender_pseudonym,
                channel_id,
                expression_id,
                actor_pseudonym,
            );
        }
        _ => {}
    }
}

/// Helper used by both `presence::voice_join_apply` and
/// `presence::voice_leave_apply` to broadcast a mesh↔MCU mode flip.
pub(super) fn broadcast_mode_switch(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    mode: &str,
    host: Option<String>,
) {
    let envelope = CommunityEnvelope::Control(ControlPayload::VoiceModeSwitch {
        channel_id: channel_id.to_string(),
        mode: mode.to_string(),
        host_pseudonym: host,
    });
    deps.send_to_mesh(community_id, &envelope);
}
