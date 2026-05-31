//! Phase 14.k — stage-channel signaling handlers.
//!
//! Implements §10.7 stage-channel semantics: only listed speakers may
//! transmit; audience may request to speak via `SpeakRequest`; speakers
//! with `MANAGE_MESSAGES` or `ADMINISTRATOR` permission see the request
//! and respond with `SpeakResponse`. Transport reconcile after any
//! stage update re-elects the relay host deterministically via
//! `topology::select_stage_host`.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::error::VoiceError;
use crate::signaling::deps::{perms, CommunityVoiceEvent, StageChannelInfo, VoiceSignalingDeps};
use crate::topology;
use crate::transport::VoiceTransport;
use crate::VoiceMode;

pub(super) fn handle_stage_update(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: String,
    topic: Option<String>,
    speakers: Vec<String>,
    moderator_pseudonym: String,
) {
    deps.update_stage_channel(
        community_id,
        &channel_id,
        topic.clone(),
        speakers.clone(),
        moderator_pseudonym.clone(),
    );

    deps.emit_event(CommunityVoiceEvent::StageUpdate {
        community_id: community_id.to_string(),
        channel_id: channel_id.clone(),
        topic,
        speakers,
        moderator_pseudonym,
    });

    let Some(transport) = deps.transport_handle() else {
        return;
    };
    let my_pseudonym = deps.my_pseudonym(community_id).unwrap_or_default();
    let cid = community_id.to_string();
    {
        let deps_task = Arc::clone(deps);
        let handle = tokio::spawn(async move {
            reconcile_stage_transport(&*deps_task, &cid, &channel_id, &transport, &my_pseudonym)
                .await;
        });
        deps.register_background_handle(handle);
    }
}

pub(super) fn handle_speak_request(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: String,
    requester_pseudonym: String,
) {
    let Some(channel_id_bytes) = deps.decode_channel_id(&channel_id) else {
        return;
    };
    let my_perms = deps.my_permissions(community_id, Some(channel_id_bytes));
    if my_perms & perms::MANAGE_MESSAGES == 0 && my_perms & perms::ADMINISTRATOR == 0 {
        return;
    }
    deps.emit_event(CommunityVoiceEvent::SpeakRequest {
        community_id: community_id.to_string(),
        channel_id,
        requester_pseudonym,
    });
}

pub(super) fn handle_speak_response(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: String,
    requester_pseudonym: String,
    granted: bool,
    moderator_pseudonym: String,
) {
    let my_pseudonym = deps.my_pseudonym(community_id);
    if my_pseudonym.as_deref() == Some(requester_pseudonym.as_str()) {
        let deps_persist = Arc::clone(deps);
        let cid = community_id.to_string();
        let ch_id = channel_id.clone();
        let handle = tokio::spawn(async move {
            deps_persist.persist_hand_raise(cid, ch_id, false).await;
        });
        deps.register_background_handle(handle);

        if granted {
            deps.set_voice_engine_muted(false);
        }
    }

    deps.emit_event(CommunityVoiceEvent::SpeakResponse {
        community_id: community_id.to_string(),
        channel_id,
        requester_pseudonym,
        granted,
        moderator_pseudonym,
    });
}

/// Deterministic stage-host election + transport reconcile. Called
/// from voice_join_apply / voice_leave_apply / handle_stage_update
/// for stage channels. Architecture §10.7: stage channels always
/// operate in MCU/relay mode regardless of participant count.
pub(super) async fn reconcile_stage_transport(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    my_pseudonym: &str,
) {
    let StageChannelInfo { speakers, .. } = deps
        .stage_channel_info(community_id, channel_id)
        .unwrap_or(StageChannelInfo {
            is_stage: false,
            speakers: Vec::new(),
            moderator: None,
        });

    let mut candidates = transport.lock().await.peer_keys();
    if !my_pseudonym.is_empty() {
        candidates.push(my_pseudonym.to_string());
    }

    let mut stage_candidates: Vec<String> = candidates
        .iter()
        .filter(|c| speakers.contains(c))
        .cloned()
        .collect();
    if stage_candidates.is_empty() {
        stage_candidates = candidates;
    }
    stage_candidates.sort();
    stage_candidates.dedup();

    let Some(host_pseudonym) = topology::select_stage_host(channel_id, &stage_candidates) else {
        return;
    };

    transport.lock().await.set_mode(VoiceMode::Mcu {
        host_pseudonym: host_pseudonym.clone(),
    });

    if host_pseudonym == my_pseudonym {
        deps.start_mcu_loop();
    } else {
        deps.stop_mcu_loop().await;
    }

    if deps.voice_engine_bound_to(community_id, channel_id) {
        let allowed_to_speak = speakers.contains(&my_pseudonym.to_string());
        deps.set_voice_engine_muted(!allowed_to_speak);
    }
}

// ── User-facing stage entry points (Phase 14.o command thinning) ────

/// User clicked "Request to speak" on the stage UI. Persists the
/// hand-raise on our own membership record + broadcasts a
/// `SpeakRequest` gossip envelope so moderators see the request.
pub async fn request_to_speak<D: VoiceSignalingDeps + ?Sized>(
    deps: &Arc<D>,
    community_id: &str,
    channel_id: &str,
) -> Result<(), VoiceError> {
    let requester_pseudonym = deps
        .my_pseudonym(community_id)
        .ok_or_else(|| VoiceError::Session("no pseudonym for community".into()))?;
    deps.persist_hand_raise(community_id.to_string(), channel_id.to_string(), true)
        .await;
    let lamport = deps.next_lamport(community_id);
    let envelope = CommunityEnvelope::Control(ControlPayload::SpeakRequest {
        channel_id: channel_id.to_string(),
        requester_pseudonym,
        lamport,
    });
    deps.send_to_mesh(community_id, &envelope);
    Ok(())
}

/// Moderator clicked Approve/Deny on a SpeakRequest. If granted:
/// rotate the voice MEK (since membership effectively changed) and
/// broadcast a StageUpdate adding the requester to `stage_speakers`.
/// Always broadcasts the `SpeakResponse` envelope so the requester
/// gets feedback.
pub async fn respond_to_speak_request<D: VoiceSignalingDeps + ?Sized>(
    deps: &Arc<D>,
    community_id: &str,
    channel_id: &str,
    requester_pseudonym: &str,
    granted: bool,
) -> Result<(), VoiceError> {
    let moderator_pseudonym = deps
        .my_pseudonym(community_id)
        .ok_or_else(|| VoiceError::Session("no pseudonym for community".into()))?;

    if granted {
        deps.rotate_voice_mek_for_membership(
            community_id.to_string(),
            channel_id.to_string(),
            requester_pseudonym.to_string(),
            true,
        )
        .await;
    }

    let lamport = deps.next_lamport(community_id);
    let response = CommunityEnvelope::Control(ControlPayload::SpeakResponse {
        channel_id: channel_id.to_string(),
        requester_pseudonym: requester_pseudonym.to_string(),
        granted,
        moderator_pseudonym: moderator_pseudonym.clone(),
        lamport,
    });
    deps.send_to_mesh(community_id, &response);

    if granted {
        let mut speakers = deps.stage_speakers(community_id, channel_id);
        if !speakers.contains(&requester_pseudonym.to_string()) {
            speakers.push(requester_pseudonym.to_string());
        }
        let stage_update = CommunityEnvelope::Control(ControlPayload::StageUpdate {
            channel_id: channel_id.to_string(),
            topic: None,
            speakers,
            moderator_pseudonym,
            lamport: lamport.saturating_add(1),
        });
        deps.send_to_mesh(community_id, &stage_update);
    }

    Ok(())
}

/// Moderator action — server-mute a member. Broadcasts `VoiceMute`
/// gossip; receivers honor it if the target_pseudonym matches their
/// own (handled in `mute::handle_voice_mute`).
pub fn server_mute_member<D: VoiceSignalingDeps + ?Sized>(
    deps: &Arc<D>,
    community_id: &str,
    channel_id: &str,
    target_pseudonym: &str,
    muted: bool,
) {
    let envelope = CommunityEnvelope::Control(ControlPayload::VoiceMute {
        channel_id: channel_id.to_string(),
        target_pseudonym: target_pseudonym.to_string(),
        muted,
    });
    deps.send_to_mesh(community_id, &envelope);
}

/// Moderator action — server-deafen a member. Same shape as
/// `server_mute_member`.
pub fn server_deafen_member<D: VoiceSignalingDeps + ?Sized>(
    deps: &Arc<D>,
    community_id: &str,
    channel_id: &str,
    target_pseudonym: &str,
    deafened: bool,
) {
    let envelope = CommunityEnvelope::Control(ControlPayload::VoiceDeafen {
        channel_id: channel_id.to_string(),
        target_pseudonym: target_pseudonym.to_string(),
        deafened,
    });
    deps.send_to_mesh(community_id, &envelope);
}
