//! `ControlPayload` (67-arm union) encoder / decoder.
//!
//! Mirrors the Rust enum in `crate::dht::community::envelope::ControlPayload`
//! against the schema in `schemas/community_envelope.capnp`. Each
//! variant has its own `write_<name>` / `read_<name>` helper so the
//! dispatchers stay short and individual variant logic is independently
//! reviewable. Helpers take the typed sub-builder/reader plus a
//! `&ControlPayload` and use `let ... else { unreachable!() }` to
//! re-extract the variant's fields. The dispatcher only routes by
//! discriminant.

use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community_envelope_capnp::{self as cap, control_payload as schema};
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;

use super::len_u32;
use super::sub_types::{
    read_bootstrap_channel_messages, read_channel_mek_delivery, read_event_info,
    read_game_server_info, read_member_info, read_member_summary, read_onboarding_answer,
    read_synced_message, read_thread_info, read_voice_roster_entry,
    write_bootstrap_channel_messages, write_channel_mek_delivery, write_event_info,
    write_game_server_info, write_member_info, write_member_summary, write_onboarding_answer,
    write_synced_message, write_thread_info, write_voice_roster_entry,
};

// ── Public API ───────────────────────────────────────────────────────

/// Encode a `ControlPayload` into a Cap'n Proto builder.
pub fn encode_control_payload(
    b: schema::Builder<'_>,
    payload: &ControlPayload,
) -> Result<(), ProtocolError> {
    let mut b = b;
    use ControlPayload as CP;
    match payload {
        CP::MemberJoinRequest { .. } => write_member_join_request(b.reborrow().init_member_join_request(), payload),
        CP::MemberLeave { .. } => write_member_leave(b.reborrow().init_member_leave(), payload),
        CP::JoinAccepted { .. } => write_join_accepted(b.reborrow().init_join_accepted(), payload),
        CP::JoinRejected { .. } => write_join_rejected(b.reborrow().init_join_rejected(), payload),
        CP::MemberJoined { .. } => write_member_joined(b.reborrow().init_member_joined(), payload),
        CP::MemberRemoved { .. } => write_member_removed(b.reborrow().init_member_removed(), payload),
        CP::Kick { .. } => write_kick(b.reborrow().init_kick(), payload),
        CP::Ban { .. } => write_ban(b.reborrow().init_ban(), payload),
        CP::Unban { .. } => write_unban(b.reborrow().init_unban(), payload),
        CP::TimeoutMember { .. } => write_timeout_member(b.reborrow().init_timeout_member(), payload),
        CP::RemoveTimeout { .. } => write_remove_timeout(b.reborrow().init_remove_timeout(), payload),
        CP::MemberTimedOut { .. } => write_member_timed_out(b.reborrow().init_member_timed_out(), payload),
        CP::MessageEdited { .. } => write_message_edited(b.reborrow().init_message_edited(), payload),
        CP::MessageDeleted { .. } => write_message_deleted(b.reborrow().init_message_deleted(), payload),
        CP::MEKRotated { .. } => write_mek_rotated(b.reborrow().init_mek_rotated(), payload),
        CP::RequestMEK { .. } => write_request_mek(b.reborrow().init_request_mek(), payload),
        CP::MekTransfer { .. } => write_mek_transfer(b.reborrow().init_mek_transfer(), payload),
        CP::MekTransferAck { .. } => write_mek_transfer_ack(b.reborrow().init_mek_transfer_ack(), payload),
        CP::RequestSegmentExpansion { .. } => write_request_segment_expansion(b.reborrow().init_request_segment_expansion(), payload),
        CP::OnboardingComplete { .. } => write_onboarding_complete(b.reborrow().init_onboarding_complete(), payload),
        CP::MemberRolesChanged { .. } => write_member_roles_changed(b.reborrow().init_member_roles_changed(), payload),
        CP::ChannelOverwriteChanged { .. } => write_channel_overwrite_changed(b.reborrow().init_channel_overwrite_changed(), payload),
        CP::ReactionAdded { .. } => write_reaction_added(b.reborrow().init_reaction_added(), payload),
        CP::ReactionRemoved { .. } => write_reaction_removed(b.reborrow().init_reaction_removed(), payload),
        CP::MessagePinned { .. } => write_message_pinned(b.reborrow().init_message_pinned(), payload),
        CP::MessageUnpinned { .. } => write_message_unpinned(b.reborrow().init_message_unpinned(), payload),
        CP::EventCreated { .. } => write_event_created(b.reborrow().init_event_created(), payload),
        CP::EventUpdated { .. } => write_event_updated(b.reborrow().init_event_updated(), payload),
        CP::EventDeleted { .. } => write_event_deleted(b.reborrow().init_event_deleted(), payload),
        CP::EventRsvpChanged { .. } => write_event_rsvp_changed(b.reborrow().init_event_rsvp_changed(), payload),
        CP::ThreadCreated { .. } => write_thread_created(b.reborrow().init_thread_created(), payload),
        CP::ThreadMessageReceived { .. } => write_thread_message_received(b.reborrow().init_thread_message_received(), payload),
        CP::ThreadArchived { .. } => write_thread_archived(b.reborrow().init_thread_archived(), payload),
        CP::GameServerAdded { .. } => write_game_server_added(b.reborrow().init_game_server_added(), payload),
        CP::GameServerRemoved { .. } => write_game_server_removed(b.reborrow().init_game_server_removed(), payload),
        CP::SubmitOnboardingAnswers { .. } => write_submit_onboarding_answers(b.reborrow().init_submit_onboarding_answers(), payload),
        CP::EventReminder { .. } => write_event_reminder(b.reborrow().init_event_reminder(), payload),
        CP::KickedNotification => b.reborrow().set_kicked_notification(()),
        CP::RaidAlert { .. } => write_raid_alert(b.reborrow().init_raid_alert(), payload),
        CP::ChannelLockdown { .. } => write_channel_lockdown(b.reborrow().init_channel_lockdown(), payload),
        CP::SystemMessage { .. } => write_system_message(b.reborrow().init_system_message(), payload),
        CP::AdminKeypairGrant { .. } => write_admin_keypair_grant(b.reborrow().init_admin_keypair_grant(), payload),
        CP::SlotKeypairGrant { .. } => write_slot_keypair_grant(b.reborrow().init_slot_keypair_grant(), payload),
        CP::GovernanceUpdated { .. } => write_governance_updated(b.reborrow().init_governance_updated(), payload),
        CP::BootstrapRequest { .. } => write_bootstrap_request(b.reborrow().init_bootstrap_request(), payload),
        CP::BootstrapResponse { .. } => write_bootstrap_response(b.reborrow().init_bootstrap_response(), payload),
        CP::SyncRequest { .. } => write_sync_request(b.reborrow().init_sync_request(), payload),
        CP::SyncResponse { .. } => write_sync_response(b.reborrow().init_sync_response(), payload),
        CP::VoiceJoin { .. } => write_voice_join(b.reborrow().init_voice_join(), payload),
        CP::VoiceLeave { .. } => write_voice_leave(b.reborrow().init_voice_leave(), payload),
        CP::VoiceModeSwitch { .. } => write_voice_mode_switch(b.reborrow().init_voice_mode_switch(), payload),
        CP::StageUpdate { .. } => write_stage_update(b.reborrow().init_stage_update(), payload),
        CP::SpeakRequest { .. } => write_speak_request(b.reborrow().init_speak_request(), payload),
        CP::SpeakResponse { .. } => write_speak_response(b.reborrow().init_speak_response(), payload),
        CP::RequestAttachment { .. } => write_request_attachment(b.reborrow().init_request_attachment(), payload),
        CP::AttachmentChunk { .. } => write_attachment_chunk(b.reborrow().init_attachment_chunk(), payload),
        CP::MultiAttachmentChunk { .. } => write_multi_attachment_chunk(b.reborrow().init_multi_attachment_chunk(), payload)?,
        CP::VoiceMute { .. } => write_voice_mute(b.reborrow().init_voice_mute(), payload),
        CP::VoiceDeafen { .. } => write_voice_deafen(b.reborrow().init_voice_deafen(), payload),
        CP::VoiceRoster { .. } => write_voice_roster(b.reborrow().init_voice_roster(), payload),
        CP::SoundboardPlay { .. } => write_soundboard_play(b.reborrow().init_soundboard_play(), payload),
        CP::VideoFragment { .. } => write_video_fragment(b.reborrow().init_video_fragment(), payload),
        CP::VideoParityFragment { .. } => write_video_parity_fragment(b.reborrow().init_video_parity_fragment(), payload),
        CP::FrameAck { .. } => write_frame_ack(b.reborrow().init_frame_ack(), payload),
        CP::KeyframeRequest { .. } => write_keyframe_request(b.reborrow().init_keyframe_request(), payload),
        CP::BandwidthEstimate { .. } => write_bandwidth_estimate(b.reborrow().init_bandwidth_estimate(), payload),
        CP::MediaCapabilities { .. } => write_media_capabilities(b.reborrow().init_media_capabilities(), payload),
        CP::TopologyChange { .. } => write_topology_change(b.reborrow().init_topology_change(), payload),
        CP::LinkPreview { .. } => write_link_preview(b.reborrow().init_link_preview(), payload),
    }
    Ok(())
}

/// Decode a `ControlPayload` from a Cap'n Proto reader.
pub fn decode_control_payload(r: schema::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    use schema::Which;
    match r.which().map_err(not_in_schema)? {
        Which::MemberJoinRequest(p) => read_member_join_request(p.map_err(|e| capnp_err(&e))?),
        Which::MemberLeave(p) => read_member_leave(p.map_err(|e| capnp_err(&e))?),
        Which::JoinAccepted(p) => read_join_accepted(p.map_err(|e| capnp_err(&e))?),
        Which::JoinRejected(p) => read_join_rejected(p.map_err(|e| capnp_err(&e))?),
        Which::MemberJoined(p) => read_member_joined(p.map_err(|e| capnp_err(&e))?),
        Which::MemberRemoved(p) => read_member_removed(p.map_err(|e| capnp_err(&e))?),
        Which::Kick(p) => read_kick(p.map_err(|e| capnp_err(&e))?),
        Which::Ban(p) => read_ban(p.map_err(|e| capnp_err(&e))?),
        Which::Unban(p) => read_unban(p.map_err(|e| capnp_err(&e))?),
        Which::TimeoutMember(p) => read_timeout_member(p.map_err(|e| capnp_err(&e))?),
        Which::RemoveTimeout(p) => read_remove_timeout(p.map_err(|e| capnp_err(&e))?),
        Which::MemberTimedOut(p) => read_member_timed_out(p.map_err(|e| capnp_err(&e))?),
        Which::MessageEdited(p) => read_message_edited(p.map_err(|e| capnp_err(&e))?),
        Which::MessageDeleted(p) => read_message_deleted(p.map_err(|e| capnp_err(&e))?),
        Which::MekRotated(p) => read_mek_rotated(p.map_err(|e| capnp_err(&e))?),
        Which::RequestMek(p) => read_request_mek(p.map_err(|e| capnp_err(&e))?),
        Which::MekTransfer(p) => read_mek_transfer(p.map_err(|e| capnp_err(&e))?),
        Which::MekTransferAck(p) => read_mek_transfer_ack(p.map_err(|e| capnp_err(&e))?),
        Which::RequestSegmentExpansion(p) => read_request_segment_expansion(p.map_err(|e| capnp_err(&e))?),
        Which::OnboardingComplete(p) => read_onboarding_complete(p.map_err(|e| capnp_err(&e))?),
        Which::MemberRolesChanged(p) => read_member_roles_changed(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelOverwriteChanged(p) => read_channel_overwrite_changed(p.map_err(|e| capnp_err(&e))?),
        Which::ReactionAdded(p) => read_reaction_added(p.map_err(|e| capnp_err(&e))?),
        Which::ReactionRemoved(p) => read_reaction_removed(p.map_err(|e| capnp_err(&e))?),
        Which::MessagePinned(p) => read_message_pinned(p.map_err(|e| capnp_err(&e))?),
        Which::MessageUnpinned(p) => read_message_unpinned(p.map_err(|e| capnp_err(&e))?),
        Which::EventCreated(p) => read_event_created(p.map_err(|e| capnp_err(&e))?),
        Which::EventUpdated(p) => read_event_updated(p.map_err(|e| capnp_err(&e))?),
        Which::EventDeleted(p) => read_event_deleted(p.map_err(|e| capnp_err(&e))?),
        Which::EventRsvpChanged(p) => read_event_rsvp_changed(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadCreated(p) => read_thread_created(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadMessageReceived(p) => read_thread_message_received(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadArchived(p) => read_thread_archived(p.map_err(|e| capnp_err(&e))?),
        Which::GameServerAdded(p) => read_game_server_added(p.map_err(|e| capnp_err(&e))?),
        Which::GameServerRemoved(p) => read_game_server_removed(p.map_err(|e| capnp_err(&e))?),
        Which::SubmitOnboardingAnswers(p) => read_submit_onboarding_answers(p.map_err(|e| capnp_err(&e))?),
        Which::EventReminder(p) => read_event_reminder(p.map_err(|e| capnp_err(&e))?),
        Which::KickedNotification(()) => Ok(ControlPayload::KickedNotification),
        Which::RaidAlert(p) => Ok(read_raid_alert(p.map_err(|e| capnp_err(&e))?)),
        Which::ChannelLockdown(p) => Ok(read_channel_lockdown(p.map_err(|e| capnp_err(&e))?)),
        Which::SystemMessage(p) => read_system_message(p.map_err(|e| capnp_err(&e))?),
        Which::AdminKeypairGrant(p) => read_admin_keypair_grant(p.map_err(|e| capnp_err(&e))?),
        Which::SlotKeypairGrant(p) => read_slot_keypair_grant(p.map_err(|e| capnp_err(&e))?),
        Which::GovernanceUpdated(p) => read_governance_updated(p.map_err(|e| capnp_err(&e))?),
        Which::BootstrapRequest(p) => read_bootstrap_request(p.map_err(|e| capnp_err(&e))?),
        Which::BootstrapResponse(p) => read_bootstrap_response(p.map_err(|e| capnp_err(&e))?),
        Which::SyncRequest(p) => read_sync_request(p.map_err(|e| capnp_err(&e))?),
        Which::SyncResponse(p) => read_sync_response(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceJoin(p) => read_voice_join(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceLeave(p) => read_voice_leave(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceModeSwitch(p) => read_voice_mode_switch(p.map_err(|e| capnp_err(&e))?),
        Which::StageUpdate(p) => read_stage_update(p.map_err(|e| capnp_err(&e))?),
        Which::SpeakRequest(p) => read_speak_request(p.map_err(|e| capnp_err(&e))?),
        Which::SpeakResponse(p) => read_speak_response(p.map_err(|e| capnp_err(&e))?),
        Which::RequestAttachment(p) => read_request_attachment(p.map_err(|e| capnp_err(&e))?),
        Which::AttachmentChunk(p) => read_attachment_chunk(p.map_err(|e| capnp_err(&e))?),
        Which::MultiAttachmentChunk(p) => read_multi_attachment_chunk(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceMute(p) => read_voice_mute(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceDeafen(p) => read_voice_deafen(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceRoster(p) => read_voice_roster(p.map_err(|e| capnp_err(&e))?),
        Which::SoundboardPlay(p) => read_soundboard_play(p.map_err(|e| capnp_err(&e))?),
        Which::VideoFragment(p) => read_video_fragment(p.map_err(|e| capnp_err(&e))?),
        Which::VideoParityFragment(p) => read_video_parity_fragment(p.map_err(|e| capnp_err(&e))?),
        Which::FrameAck(p) => read_frame_ack(p.map_err(|e| capnp_err(&e))?),
        Which::KeyframeRequest(p) => read_keyframe_request(p.map_err(|e| capnp_err(&e))?),
        Which::BandwidthEstimate(p) => read_bandwidth_estimate(p.map_err(|e| capnp_err(&e))?),
        Which::MediaCapabilities(p) => read_media_capabilities(p.map_err(|e| capnp_err(&e))?),
        Which::TopologyChange(p) => read_topology_change(p.map_err(|e| capnp_err(&e))?),
        Which::LinkPreview(p) => read_link_preview(p.map_err(|e| capnp_err(&e))?),
    }
}

// ── Per-variant write helpers ────────────────────────────────────────
//
// Each `write_<name>` re-extracts its variant via `let ... else
// { unreachable!() }` so the dispatcher can stay short. The dispatcher
// guarantees only the matching variant is passed.

fn write_member_join_request(mut p: cap::member_join_request_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MemberJoinRequest { pseudonym_key, display_name, invite_code, route_blob, prekey_bundle, claimed_subkey_index } = payload else {
        unreachable!("write_member_join_request: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
    p.set_display_name(display_name);
    p.set_has_invite_code(invite_code.is_some());
    if let Some(c) = invite_code { p.set_invite_code(c); }
    p.set_has_route_blob(route_blob.is_some());
    if let Some(rb) = route_blob { p.set_route_blob(rb); }
    p.set_has_prekey_bundle(prekey_bundle.is_some());
    if let Some(pb) = prekey_bundle { p.set_prekey_bundle(pb); }
    p.set_has_claimed_subkey(claimed_subkey_index.is_some());
    if let Some(idx) = claimed_subkey_index { p.set_claimed_subkey_index(*idx); }
}

fn write_member_leave(mut p: cap::member_leave_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MemberLeave { pseudonym_key } = payload else {
        unreachable!("write_member_leave: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
}

fn write_join_accepted(mut p: cap::join_accepted_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::JoinAccepted { mek_encrypted, mek_generation, members, member_registry_key, slot_index, wrapped_slot_seed } = payload else {
        unreachable!("write_join_accepted: variant mismatch")
    };
    p.set_mek_encrypted(mek_encrypted);
    p.set_mek_generation(*mek_generation);
    let mut list = p.reborrow().init_members(len_u32(members.len()));
    for (i, m) in members.iter().enumerate() {
        write_member_summary(list.reborrow().get(len_u32(i)), m);
    }
    p.set_has_member_registry_key(member_registry_key.is_some());
    if let Some(k) = member_registry_key { p.set_member_registry_key(k); }
    p.set_has_slot_index(slot_index.is_some());
    if let Some(idx) = slot_index { p.set_slot_index(*idx); }
    p.set_has_wrapped_slot_seed(wrapped_slot_seed.is_some());
    if let Some(seed) = wrapped_slot_seed { p.set_wrapped_slot_seed(seed); }
}

fn write_join_rejected(mut p: cap::join_rejected_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::JoinRejected { reason } = payload else {
        unreachable!("write_join_rejected: variant mismatch")
    };
    p.set_reason(reason);
}

fn write_member_joined(mut p: cap::member_joined_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MemberJoined { pseudonym_key, display_name, role_ids, status, route_blob } = payload else {
        unreachable!("write_member_joined: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
    p.set_display_name(display_name);
    let mut list = p.reborrow().init_role_ids(len_u32(role_ids.len()));
    for (i, id) in role_ids.iter().enumerate() {
        list.set(len_u32(i), *id);
    }
    p.set_status(status);
    p.set_has_route_blob(route_blob.is_some());
    if let Some(rb) = route_blob { p.set_route_blob(rb); }
}

fn write_member_removed(mut p: cap::member_removed_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MemberRemoved { pseudonym_key } = payload else {
        unreachable!("write_member_removed: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
}

fn write_kick(mut p: cap::kick_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::Kick { target_pseudonym } = payload else {
        unreachable!("write_kick: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

fn write_ban(mut p: cap::ban_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::Ban { target_pseudonym } = payload else {
        unreachable!("write_ban: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

fn write_unban(mut p: cap::unban_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::Unban { target_pseudonym } = payload else {
        unreachable!("write_unban: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

fn write_timeout_member(mut p: cap::timeout_member_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::TimeoutMember { target_pseudonym, duration_seconds, reason } = payload else {
        unreachable!("write_timeout_member: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
    p.set_duration_seconds(*duration_seconds);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason { p.set_reason(r); }
}

fn write_remove_timeout(mut p: cap::remove_timeout_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::RemoveTimeout { target_pseudonym } = payload else {
        unreachable!("write_remove_timeout: variant mismatch")
    };
    p.set_target_pseudonym(target_pseudonym);
}

fn write_member_timed_out(mut p: cap::member_timed_out_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MemberTimedOut { pseudonym_key, timeout_until } = payload else {
        unreachable!("write_member_timed_out: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
    p.set_has_timeout_until(timeout_until.is_some());
    if let Some(t) = timeout_until { p.set_timeout_until(*t); }
}

fn write_message_edited(mut p: cap::message_edited_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MessageEdited { channel_id, message_id, new_ciphertext, mek_generation, edited_at } = payload else {
        unreachable!("write_message_edited: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_new_ciphertext(new_ciphertext);
    p.set_mek_generation(*mek_generation);
    p.set_edited_at(*edited_at);
}

fn write_message_deleted(mut p: cap::message_deleted_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MessageDeleted { channel_id, message_id } = payload else {
        unreachable!("write_message_deleted: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
}

fn write_mek_rotated(mut p: cap::m_e_k_rotated_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MEKRotated { channel_id, new_generation, rotator_pseudonym } = payload else {
        unreachable!("write_mek_rotated: variant mismatch")
    };
    p.set_has_channel_id(channel_id.is_some());
    if let Some(c) = channel_id { p.set_channel_id(c); }
    p.set_new_generation(*new_generation);
    p.set_has_rotator_pseudonym(rotator_pseudonym.is_some());
    if let Some(rp) = rotator_pseudonym { p.set_rotator_pseudonym(rp); }
}

fn write_request_mek(mut p: cap::request_m_e_k_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::RequestMEK { channel_id, needed_generation, requester_pseudonym, cascade_index } = payload else {
        unreachable!("write_request_mek: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_needed_generation(*needed_generation);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_cascade_index(*cascade_index);
}

fn write_mek_transfer(mut p: cap::mek_transfer_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MekTransfer { community_id, channel_id, generation, sender_pseudonym, wrapped_mek } = payload else {
        unreachable!("write_mek_transfer: variant mismatch")
    };
    p.set_community_id(community_id);
    p.set_has_channel_id(channel_id.is_some());
    if let Some(c) = channel_id { p.set_channel_id(c); }
    p.set_generation(*generation);
    p.set_sender_pseudonym(sender_pseudonym);
    p.set_wrapped_mek(wrapped_mek);
}

fn write_mek_transfer_ack(mut p: cap::mek_transfer_ack_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MekTransferAck { community_id, channel_id, generation, requester_pseudonym } = payload else {
        unreachable!("write_mek_transfer_ack: variant mismatch")
    };
    p.set_community_id(community_id);
    p.set_has_channel_id(channel_id.is_some());
    if let Some(c) = channel_id { p.set_channel_id(c); }
    p.set_generation(*generation);
    p.set_requester_pseudonym(requester_pseudonym);
}

fn write_request_segment_expansion(mut p: cap::request_segment_expansion_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::RequestSegmentExpansion { community_id, requester_pseudonym, full_segment_index } = payload else {
        unreachable!("write_request_segment_expansion: variant mismatch")
    };
    p.set_community_id(community_id);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_full_segment_index(*full_segment_index);
}

fn write_onboarding_complete(mut p: cap::onboarding_complete_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::OnboardingComplete { pseudonym_key, role_ids } = payload else {
        unreachable!("write_onboarding_complete: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
    let mut list = p.reborrow().init_role_ids(len_u32(role_ids.len()));
    for (i, id) in role_ids.iter().enumerate() { list.set(len_u32(i), *id); }
}

fn write_member_roles_changed(mut p: cap::member_roles_changed_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MemberRolesChanged { pseudonym_key, role_ids } = payload else {
        unreachable!("write_member_roles_changed: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
    let mut list = p.reborrow().init_role_ids(len_u32(role_ids.len()));
    for (i, id) in role_ids.iter().enumerate() { list.set(len_u32(i), *id); }
}

fn write_channel_overwrite_changed(mut p: cap::channel_overwrite_changed_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::ChannelOverwriteChanged { channel_id } = payload else {
        unreachable!("write_channel_overwrite_changed: variant mismatch")
    };
    p.set_channel_id(channel_id);
}

fn write_reaction_added(mut p: cap::reaction_added_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::ReactionAdded { channel_id, message_id, emoji, reactor_pseudonym } = payload else {
        unreachable!("write_reaction_added: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_emoji(emoji);
    p.set_reactor_pseudonym(reactor_pseudonym);
}

fn write_reaction_removed(mut p: cap::reaction_removed_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::ReactionRemoved { channel_id, message_id, emoji, reactor_pseudonym } = payload else {
        unreachable!("write_reaction_removed: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_emoji(emoji);
    p.set_reactor_pseudonym(reactor_pseudonym);
}

fn write_message_pinned(mut p: cap::message_pinned_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MessagePinned { channel_id, message_id, pinned_by } = payload else {
        unreachable!("write_message_pinned: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_pinned_by(pinned_by);
}

fn write_message_unpinned(mut p: cap::message_unpinned_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MessageUnpinned { channel_id, message_id } = payload else {
        unreachable!("write_message_unpinned: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
}

fn write_event_created(p: cap::event_created_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::EventCreated { event } = payload else {
        unreachable!("write_event_created: variant mismatch")
    };
    write_event_info(p.init_event(), event);
}

fn write_event_updated(p: cap::event_updated_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::EventUpdated { event } = payload else {
        unreachable!("write_event_updated: variant mismatch")
    };
    write_event_info(p.init_event(), event);
}

fn write_event_deleted(mut p: cap::event_deleted_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::EventDeleted { event_id } = payload else {
        unreachable!("write_event_deleted: variant mismatch")
    };
    p.set_event_id(event_id);
}

fn write_event_rsvp_changed(mut p: cap::event_rsvp_changed_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::EventRsvpChanged { event_id, pseudonym_key, status } = payload else {
        unreachable!("write_event_rsvp_changed: variant mismatch")
    };
    p.set_event_id(event_id);
    p.set_pseudonym_key(pseudonym_key);
    p.set_status(status);
}

fn write_thread_created(p: cap::thread_created_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::ThreadCreated { thread } = payload else {
        unreachable!("write_thread_created: variant mismatch")
    };
    write_thread_info(p.init_thread(), thread);
}

fn write_thread_message_received(mut p: cap::thread_message_received_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::ThreadMessageReceived { thread_id, message_id, sender_pseudonym, ciphertext, mek_generation, timestamp, reply_to_id } = payload else {
        unreachable!("write_thread_message_received: variant mismatch")
    };
    p.set_thread_id(thread_id);
    p.set_message_id(message_id);
    p.set_sender_pseudonym(sender_pseudonym);
    p.set_ciphertext(ciphertext);
    p.set_mek_generation(*mek_generation);
    p.set_timestamp(*timestamp);
    p.set_has_reply_to_id(reply_to_id.is_some());
    if let Some(r) = reply_to_id { p.set_reply_to_id(r); }
}

fn write_thread_archived(mut p: cap::thread_archived_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::ThreadArchived { thread_id, archived } = payload else {
        unreachable!("write_thread_archived: variant mismatch")
    };
    p.set_thread_id(thread_id);
    p.set_archived(*archived);
}

fn write_game_server_added(p: cap::game_server_added_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::GameServerAdded { server } = payload else {
        unreachable!("write_game_server_added: variant mismatch")
    };
    write_game_server_info(p.init_server(), server);
}

fn write_game_server_removed(mut p: cap::game_server_removed_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::GameServerRemoved { server_id } = payload else {
        unreachable!("write_game_server_removed: variant mismatch")
    };
    p.set_server_id(server_id);
}

fn write_submit_onboarding_answers(mut p: cap::submit_onboarding_answers_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::SubmitOnboardingAnswers { answers } = payload else {
        unreachable!("write_submit_onboarding_answers: variant mismatch")
    };
    let mut list = p.reborrow().init_answers(len_u32(answers.len()));
    for (i, a) in answers.iter().enumerate() {
        write_onboarding_answer(list.reborrow().get(len_u32(i)), a);
    }
}

fn write_event_reminder(mut p: cap::event_reminder_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::EventReminder { event_id, title, minutes_until_start } = payload else {
        unreachable!("write_event_reminder: variant mismatch")
    };
    p.set_event_id(event_id);
    p.set_title(title);
    p.set_minutes_until_start(*minutes_until_start);
}

fn write_raid_alert(mut p: cap::raid_alert_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::RaidAlert { active } = payload else {
        unreachable!("write_raid_alert: variant mismatch")
    };
    p.set_active(*active);
}

fn write_channel_lockdown(mut p: cap::channel_lockdown_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::ChannelLockdown { locked } = payload else {
        unreachable!("write_channel_lockdown: variant mismatch")
    };
    p.set_locked(*locked);
}

fn write_system_message(mut p: cap::system_message_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::SystemMessage { body, timestamp } = payload else {
        unreachable!("write_system_message: variant mismatch")
    };
    p.set_body(body);
    p.set_timestamp(*timestamp);
}

fn write_admin_keypair_grant(mut p: cap::admin_keypair_grant_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::AdminKeypairGrant { wrapped_owner_keypair, wrapped_slot_seed } = payload else {
        unreachable!("write_admin_keypair_grant: variant mismatch")
    };
    p.set_wrapped_owner_keypair(wrapped_owner_keypair);
    p.set_wrapped_slot_seed(wrapped_slot_seed);
}

fn write_slot_keypair_grant(mut p: cap::slot_keypair_grant_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::SlotKeypairGrant { slot_index, segment_index, wrapped_slot_keypair } = payload else {
        unreachable!("write_slot_keypair_grant: variant mismatch")
    };
    p.set_slot_index(*slot_index);
    p.set_segment_index(*segment_index);
    p.set_wrapped_slot_keypair(wrapped_slot_keypair);
}

fn write_governance_updated(mut p: cap::governance_updated_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::GovernanceUpdated { governance_key, subkey_index, lamport_ts } = payload else {
        unreachable!("write_governance_updated: variant mismatch")
    };
    p.set_governance_key(governance_key);
    p.set_subkey_index(*subkey_index);
    p.set_lamport_ts(*lamport_ts);
}

fn write_bootstrap_request(mut p: cap::bootstrap_request_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::BootstrapRequest { joiner_pseudonym, governance_key } = payload else {
        unreachable!("write_bootstrap_request: variant mismatch")
    };
    p.set_joiner_pseudonym(joiner_pseudonym);
    p.set_governance_key(governance_key);
}

fn write_bootstrap_response(mut p: cap::bootstrap_response_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::BootstrapResponse { governance_entries, member_list, channel_meks, recent_messages, wrapped_owner_keypair } = payload else {
        unreachable!("write_bootstrap_response: variant mismatch")
    };
    let mut entries = p.reborrow().init_governance_entries(len_u32(governance_entries.len()));
    for (i, e) in governance_entries.iter().enumerate() {
        super::governance::write_governance_entry(entries.reborrow().get(len_u32(i)), e);
    }
    let mut members = p.reborrow().init_member_list(len_u32(member_list.len()));
    for (i, m) in member_list.iter().enumerate() {
        write_member_info(members.reborrow().get(len_u32(i)), m);
    }
    let mut meks = p.reborrow().init_channel_meks(len_u32(channel_meks.len()));
    for (i, m) in channel_meks.iter().enumerate() {
        write_channel_mek_delivery(meks.reborrow().get(len_u32(i)), m);
    }
    let mut msgs = p.reborrow().init_recent_messages(len_u32(recent_messages.len()));
    for (i, m) in recent_messages.iter().enumerate() {
        write_bootstrap_channel_messages(msgs.reborrow().get(len_u32(i)), m);
    }
    p.set_wrapped_owner_keypair(wrapped_owner_keypair);
}

fn write_sync_request(mut p: cap::sync_request_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::SyncRequest { channel_id, since_timestamp } = payload else {
        unreachable!("write_sync_request: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_since_timestamp(*since_timestamp);
}

fn write_sync_response(mut p: cap::sync_response_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::SyncResponse { channel_id, messages } = payload else {
        unreachable!("write_sync_response: variant mismatch")
    };
    p.set_channel_id(channel_id);
    let mut list = p.reborrow().init_messages(len_u32(messages.len()));
    for (i, m) in messages.iter().enumerate() {
        write_synced_message(list.reborrow().get(len_u32(i)), m);
    }
}

fn write_voice_join(mut p: cap::voice_join_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::VoiceJoin { channel_id, route_blob } = payload else {
        unreachable!("write_voice_join: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_route_blob(route_blob);
}

fn write_voice_leave(mut p: cap::voice_leave_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::VoiceLeave { channel_id } = payload else {
        unreachable!("write_voice_leave: variant mismatch")
    };
    p.set_channel_id(channel_id);
}

fn write_voice_mode_switch(mut p: cap::voice_mode_switch_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::VoiceModeSwitch { channel_id, mode, host_pseudonym } = payload else {
        unreachable!("write_voice_mode_switch: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_mode(mode);
    p.set_has_host_pseudonym(host_pseudonym.is_some());
    if let Some(h) = host_pseudonym { p.set_host_pseudonym(h); }
}

fn write_stage_update(mut p: cap::stage_update_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::StageUpdate { channel_id, topic, speakers, moderator_pseudonym, lamport } = payload else {
        unreachable!("write_stage_update: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_has_topic(topic.is_some());
    if let Some(t) = topic { p.set_topic(t); }
    let mut list = p.reborrow().init_speakers(len_u32(speakers.len()));
    for (i, s) in speakers.iter().enumerate() { list.set(len_u32(i), s.as_str()); }
    p.set_moderator_pseudonym(moderator_pseudonym);
    p.set_lamport(*lamport);
}

fn write_speak_request(mut p: cap::speak_request_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::SpeakRequest { channel_id, requester_pseudonym, lamport } = payload else {
        unreachable!("write_speak_request: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_lamport(*lamport);
}

fn write_speak_response(mut p: cap::speak_response_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::SpeakResponse { channel_id, requester_pseudonym, granted, moderator_pseudonym, lamport } = payload else {
        unreachable!("write_speak_response: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_granted(*granted);
    p.set_moderator_pseudonym(moderator_pseudonym);
    p.set_lamport(*lamport);
}

fn write_request_attachment(mut p: cap::request_attachment_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::RequestAttachment { channel_id, attachment_id, requested_chunks, requester_pseudonym } = payload else {
        unreachable!("write_request_attachment: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_attachment_id(attachment_id);
    let mut list = p.reborrow().init_requested_chunks(len_u32(requested_chunks.len()));
    for (i, c) in requested_chunks.iter().enumerate() { list.set(len_u32(i), *c); }
    p.set_requester_pseudonym(requester_pseudonym);
}

fn write_attachment_chunk(mut p: cap::attachment_chunk_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::AttachmentChunk { attachment_id, chunk_index, data, plaintext_hash } = payload else {
        unreachable!("write_attachment_chunk: variant mismatch")
    };
    p.set_attachment_id(attachment_id);
    p.set_chunk_index(*chunk_index);
    p.set_data(data);
    p.set_plaintext_hash(plaintext_hash);
}

fn write_multi_attachment_chunk(
    mut p: cap::multi_attachment_chunk_payload::Builder<'_>,
    payload: &ControlPayload,
) -> Result<(), ProtocolError> {
    let ControlPayload::MultiAttachmentChunk { chunks } = payload else {
        unreachable!("write_multi_attachment_chunk: variant mismatch")
    };
    let mut list = p.reborrow().init_chunks(len_u32(chunks.len()));
    for (i, c) in chunks.iter().enumerate() {
        encode_control_payload(list.reborrow().get(len_u32(i)), c)?;
    }
    Ok(())
}

fn write_voice_mute(mut p: cap::voice_mute_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::VoiceMute { channel_id, target_pseudonym, muted } = payload else {
        unreachable!("write_voice_mute: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_target_pseudonym(target_pseudonym);
    p.set_muted(*muted);
}

fn write_voice_deafen(mut p: cap::voice_deafen_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::VoiceDeafen { channel_id, target_pseudonym, deafened } = payload else {
        unreachable!("write_voice_deafen: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_target_pseudonym(target_pseudonym);
    p.set_deafened(*deafened);
}

fn write_voice_roster(mut p: cap::voice_roster_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::VoiceRoster { channel_id, participants } = payload else {
        unreachable!("write_voice_roster: variant mismatch")
    };
    p.set_channel_id(channel_id);
    let mut list = p.reborrow().init_participants(len_u32(participants.len()));
    for (i, e) in participants.iter().enumerate() {
        write_voice_roster_entry(list.reborrow().get(len_u32(i)), e);
    }
}

fn write_soundboard_play(mut p: cap::soundboard_play_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::SoundboardPlay { channel_id, expression_id, actor_pseudonym } = payload else {
        unreachable!("write_soundboard_play: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_expression_id(expression_id);
    p.set_actor_pseudonym(actor_pseudonym);
}

fn write_video_fragment(mut p: cap::video_fragment_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::VideoFragment { channel_id, stream_id, frame_seq, frag_index, frag_total, keyframe, timestamp, payload: data, signature } = payload else {
        unreachable!("write_video_fragment: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
    p.set_frame_seq(*frame_seq);
    p.set_frag_index(*frag_index);
    p.set_frag_total(*frag_total);
    p.set_keyframe(*keyframe);
    p.set_timestamp(*timestamp);
    p.set_payload(data);
    p.set_signature(signature);
}

fn write_video_parity_fragment(mut p: cap::video_parity_fragment_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::VideoParityFragment { channel_id, stream_id, frame_seq, parity_index, parity_total, data_count, frame_len, timestamp, payload: data, signature } = payload else {
        unreachable!("write_video_parity_fragment: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
    p.set_frame_seq(*frame_seq);
    p.set_parity_index(*parity_index);
    p.set_parity_total(*parity_total);
    p.set_data_count(*data_count);
    p.set_frame_len(*frame_len);
    p.set_timestamp(*timestamp);
    p.set_payload(data);
    p.set_signature(signature);
}

fn write_frame_ack(mut p: cap::frame_ack_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::FrameAck { channel_id, stream_id, last_frame_seq, kbps, loss_q8 } = payload else {
        unreachable!("write_frame_ack: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
    p.set_last_frame_seq(*last_frame_seq);
    p.set_kbps(*kbps);
    p.set_loss_q8(*loss_q8);
}

fn write_keyframe_request(mut p: cap::keyframe_request_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::KeyframeRequest { channel_id, stream_id } = payload else {
        unreachable!("write_keyframe_request: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
}

fn write_bandwidth_estimate(mut p: cap::bandwidth_estimate_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::BandwidthEstimate { channel_id, kbps, window_secs, loss_q8 } = payload else {
        unreachable!("write_bandwidth_estimate: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_kbps(*kbps);
    p.set_window_secs(*window_secs);
    p.set_loss_q8(*loss_q8);
}

fn write_media_capabilities(mut p: cap::media_capabilities_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::MediaCapabilities { channel_id, max_pixel_count, max_fps, codecs } = payload else {
        unreachable!("write_media_capabilities: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_max_pixel_count(*max_pixel_count);
    p.set_max_fps(*max_fps);
    let mut list = p.reborrow().init_codecs(len_u32(codecs.len()));
    for (i, c) in codecs.iter().enumerate() { list.set(len_u32(i), c.as_str()); }
}

fn write_topology_change(mut p: cap::topology_change_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::TopologyChange { channel_id, stream_id, relay_host_pseudonym, reason, lamport } = payload else {
        unreachable!("write_topology_change: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_stream_id(stream_id);
    p.set_has_relay_host(relay_host_pseudonym.is_some());
    if let Some(rh) = relay_host_pseudonym { p.set_relay_host_pseudonym(rh); }
    p.set_reason(reason);
    p.set_lamport(*lamport);
}

fn write_link_preview(mut p: cap::link_preview_payload::Builder<'_>, payload: &ControlPayload) {
    let ControlPayload::LinkPreview { channel_id, message_id, url, title, description, image_url, site_name, fetched_at } = payload else {
        unreachable!("write_link_preview: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_message_id(message_id);
    p.set_url(url);
    p.set_has_title(title.is_some());
    if let Some(t) = title { p.set_title(t); }
    p.set_has_description(description.is_some());
    if let Some(d) = description { p.set_description(d); }
    p.set_has_image_url(image_url.is_some());
    if let Some(u) = image_url { p.set_image_url(u); }
    p.set_has_site_name(site_name.is_some());
    if let Some(s) = site_name { p.set_site_name(s); }
    p.set_fetched_at(*fetched_at);
}

// ── Per-variant read helpers ─────────────────────────────────────────

fn read_member_join_request(p: cap::member_join_request_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MemberJoinRequest {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        display_name: text_to_string(p.get_display_name().map_err(|e| capnp_err(&e))?)?,
        invite_code: if p.get_has_invite_code() {
            Some(text_to_string(p.get_invite_code().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        route_blob: if p.get_has_route_blob() {
            Some(p.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec())
        } else { None },
        prekey_bundle: if p.get_has_prekey_bundle() {
            Some(p.get_prekey_bundle().map_err(|e| capnp_err(&e))?.to_vec())
        } else { None },
        claimed_subkey_index: if p.get_has_claimed_subkey() {
            Some(p.get_claimed_subkey_index())
        } else { None },
    })
}

fn read_member_leave(p: cap::member_leave_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MemberLeave {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_join_accepted(p: cap::join_accepted_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let members: Result<Vec<_>, ProtocolError> = p
        .get_members()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_member_summary)
        .collect();
    Ok(ControlPayload::JoinAccepted {
        mek_encrypted: p.get_mek_encrypted().map_err(|e| capnp_err(&e))?.to_vec(),
        mek_generation: p.get_mek_generation(),
        members: members?,
        member_registry_key: if p.get_has_member_registry_key() {
            Some(text_to_string(p.get_member_registry_key().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        slot_index: if p.get_has_slot_index() { Some(p.get_slot_index()) } else { None },
        wrapped_slot_seed: if p.get_has_wrapped_slot_seed() {
            Some(p.get_wrapped_slot_seed().map_err(|e| capnp_err(&e))?.to_vec())
        } else { None },
    })
}

fn read_join_rejected(p: cap::join_rejected_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::JoinRejected {
        reason: text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_member_joined(p: cap::member_joined_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let role_ids: Vec<u32> = p.get_role_ids().map_err(|e| capnp_err(&e))?.iter().collect();
    Ok(ControlPayload::MemberJoined {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        display_name: text_to_string(p.get_display_name().map_err(|e| capnp_err(&e))?)?,
        role_ids,
        status: text_to_string(p.get_status().map_err(|e| capnp_err(&e))?)?,
        route_blob: if p.get_has_route_blob() {
            Some(p.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec())
        } else { None },
    })
}

fn read_member_removed(p: cap::member_removed_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MemberRemoved {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_kick(p: cap::kick_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::Kick {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_ban(p: cap::ban_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::Ban {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_unban(p: cap::unban_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::Unban {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_timeout_member(p: cap::timeout_member_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::TimeoutMember {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
        duration_seconds: p.get_duration_seconds(),
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else { None },
    })
}

fn read_remove_timeout(p: cap::remove_timeout_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::RemoveTimeout {
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_member_timed_out(p: cap::member_timed_out_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MemberTimedOut {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        timeout_until: if p.get_has_timeout_until() { Some(p.get_timeout_until()) } else { None },
    })
}

fn read_message_edited(p: cap::message_edited_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MessageEdited {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        new_ciphertext: p.get_new_ciphertext().map_err(|e| capnp_err(&e))?.to_vec(),
        mek_generation: p.get_mek_generation(),
        edited_at: p.get_edited_at(),
    })
}

fn read_message_deleted(p: cap::message_deleted_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MessageDeleted {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_mek_rotated(p: cap::m_e_k_rotated_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MEKRotated {
        channel_id: if p.get_has_channel_id() {
            Some(text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        new_generation: p.get_new_generation(),
        rotator_pseudonym: if p.get_has_rotator_pseudonym() {
            Some(text_to_string(p.get_rotator_pseudonym().map_err(|e| capnp_err(&e))?)?)
        } else { None },
    })
}

fn read_request_mek(p: cap::request_m_e_k_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::RequestMEK {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        needed_generation: p.get_needed_generation(),
        requester_pseudonym: text_to_string(p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?)?,
        // Cap'n Proto fills missing fields with zero on read, so peers
        // pre-dating cascadeIndex transparently produce cascade_index=0
        // (the deterministic top-rank responder).
        cascade_index: p.get_cascade_index(),
    })
}

fn read_mek_transfer(p: cap::mek_transfer_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MekTransfer {
        community_id: text_to_string(p.get_community_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: if p.get_has_channel_id() {
            Some(text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        generation: p.get_generation(),
        sender_pseudonym: text_to_string(p.get_sender_pseudonym().map_err(|e| capnp_err(&e))?)?,
        wrapped_mek: p.get_wrapped_mek().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

fn read_mek_transfer_ack(p: cap::mek_transfer_ack_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MekTransferAck {
        community_id: text_to_string(p.get_community_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: if p.get_has_channel_id() {
            Some(text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        generation: p.get_generation(),
        requester_pseudonym: text_to_string(p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_request_segment_expansion(p: cap::request_segment_expansion_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::RequestSegmentExpansion {
        community_id: text_to_string(p.get_community_id().map_err(|e| capnp_err(&e))?)?,
        requester_pseudonym: text_to_string(p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?)?,
        full_segment_index: p.get_full_segment_index(),
    })
}

fn read_onboarding_complete(p: cap::onboarding_complete_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let role_ids: Vec<u32> = p.get_role_ids().map_err(|e| capnp_err(&e))?.iter().collect();
    Ok(ControlPayload::OnboardingComplete {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        role_ids,
    })
}

fn read_member_roles_changed(p: cap::member_roles_changed_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let role_ids: Vec<u32> = p.get_role_ids().map_err(|e| capnp_err(&e))?.iter().collect();
    Ok(ControlPayload::MemberRolesChanged {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        role_ids,
    })
}

fn read_channel_overwrite_changed(p: cap::channel_overwrite_changed_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ChannelOverwriteChanged {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_reaction_added(p: cap::reaction_added_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ReactionAdded {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        emoji: text_to_string(p.get_emoji().map_err(|e| capnp_err(&e))?)?,
        reactor_pseudonym: text_to_string(p.get_reactor_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_reaction_removed(p: cap::reaction_removed_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ReactionRemoved {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        emoji: text_to_string(p.get_emoji().map_err(|e| capnp_err(&e))?)?,
        reactor_pseudonym: text_to_string(p.get_reactor_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_message_pinned(p: cap::message_pinned_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MessagePinned {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        pinned_by: text_to_string(p.get_pinned_by().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_message_unpinned(p: cap::message_unpinned_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MessageUnpinned {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_event_created(p: cap::event_created_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventCreated {
        event: read_event_info(p.get_event().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_event_updated(p: cap::event_updated_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventUpdated {
        event: read_event_info(p.get_event().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_event_deleted(p: cap::event_deleted_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventDeleted {
        event_id: text_to_string(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_event_rsvp_changed(p: cap::event_rsvp_changed_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventRsvpChanged {
        event_id: text_to_string(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        status: text_to_string(p.get_status().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_thread_created(p: cap::thread_created_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ThreadCreated {
        thread: read_thread_info(p.get_thread().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_thread_message_received(p: cap::thread_message_received_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ThreadMessageReceived {
        thread_id: text_to_string(p.get_thread_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        sender_pseudonym: text_to_string(p.get_sender_pseudonym().map_err(|e| capnp_err(&e))?)?,
        ciphertext: p.get_ciphertext().map_err(|e| capnp_err(&e))?.to_vec(),
        mek_generation: p.get_mek_generation(),
        timestamp: p.get_timestamp(),
        reply_to_id: if p.get_has_reply_to_id() {
            Some(text_to_string(p.get_reply_to_id().map_err(|e| capnp_err(&e))?)?)
        } else { None },
    })
}

fn read_thread_archived(p: cap::thread_archived_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ThreadArchived {
        thread_id: text_to_string(p.get_thread_id().map_err(|e| capnp_err(&e))?)?,
        archived: p.get_archived(),
    })
}

fn read_game_server_added(p: cap::game_server_added_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::GameServerAdded {
        server: read_game_server_info(p.get_server().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_game_server_removed(p: cap::game_server_removed_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::GameServerRemoved {
        server_id: text_to_string(p.get_server_id().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_submit_onboarding_answers(p: cap::submit_onboarding_answers_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let answers: Result<Vec<_>, ProtocolError> = p
        .get_answers()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_onboarding_answer)
        .collect();
    Ok(ControlPayload::SubmitOnboardingAnswers { answers: answers? })
}

fn read_event_reminder(p: cap::event_reminder_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::EventReminder {
        event_id: text_to_string(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
        title: text_to_string(p.get_title().map_err(|e| capnp_err(&e))?)?,
        minutes_until_start: p.get_minutes_until_start(),
    })
}

fn read_raid_alert(p: cap::raid_alert_payload::Reader<'_>) -> ControlPayload {
    ControlPayload::RaidAlert { active: p.get_active() }
}

fn read_channel_lockdown(p: cap::channel_lockdown_payload::Reader<'_>) -> ControlPayload {
    ControlPayload::ChannelLockdown { locked: p.get_locked() }
}

fn read_system_message(p: cap::system_message_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SystemMessage {
        body: text_to_string(p.get_body().map_err(|e| capnp_err(&e))?)?,
        timestamp: p.get_timestamp(),
    })
}

fn read_admin_keypair_grant(p: cap::admin_keypair_grant_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::AdminKeypairGrant {
        wrapped_owner_keypair: p.get_wrapped_owner_keypair().map_err(|e| capnp_err(&e))?.to_vec(),
        wrapped_slot_seed: p.get_wrapped_slot_seed().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

fn read_slot_keypair_grant(p: cap::slot_keypair_grant_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SlotKeypairGrant {
        slot_index: p.get_slot_index(),
        segment_index: p.get_segment_index(),
        wrapped_slot_keypair: p.get_wrapped_slot_keypair().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

fn read_governance_updated(p: cap::governance_updated_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::GovernanceUpdated {
        governance_key: text_to_string(p.get_governance_key().map_err(|e| capnp_err(&e))?)?,
        subkey_index: p.get_subkey_index(),
        lamport_ts: p.get_lamport_ts(),
    })
}

fn read_bootstrap_request(p: cap::bootstrap_request_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::BootstrapRequest {
        joiner_pseudonym: text_to_string(p.get_joiner_pseudonym().map_err(|e| capnp_err(&e))?)?,
        governance_key: text_to_string(p.get_governance_key().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_bootstrap_response(p: cap::bootstrap_response_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let governance_entries: Result<Vec<_>, ProtocolError> = p
        .get_governance_entries()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(super::governance::read_governance_entry)
        .collect();
    let member_list: Result<Vec<_>, ProtocolError> = p
        .get_member_list()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_member_info)
        .collect();
    let channel_meks: Result<Vec<_>, ProtocolError> = p
        .get_channel_meks()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_channel_mek_delivery)
        .collect();
    let recent_messages: Result<Vec<_>, ProtocolError> = p
        .get_recent_messages()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_bootstrap_channel_messages)
        .collect();
    Ok(ControlPayload::BootstrapResponse {
        governance_entries: governance_entries?,
        member_list: member_list?,
        channel_meks: channel_meks?,
        recent_messages: recent_messages?,
        wrapped_owner_keypair: p.get_wrapped_owner_keypair().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

fn read_sync_request(p: cap::sync_request_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SyncRequest {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        since_timestamp: p.get_since_timestamp(),
    })
}

fn read_sync_response(p: cap::sync_response_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let messages: Result<Vec<_>, ProtocolError> = p
        .get_messages()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_synced_message)
        .collect();
    Ok(ControlPayload::SyncResponse {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        messages: messages?,
    })
}

fn read_voice_join(p: cap::voice_join_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceJoin {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        route_blob: p.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

fn read_voice_leave(p: cap::voice_leave_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceLeave {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_voice_mode_switch(p: cap::voice_mode_switch_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceModeSwitch {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        mode: text_to_string(p.get_mode().map_err(|e| capnp_err(&e))?)?,
        host_pseudonym: if p.get_has_host_pseudonym() {
            Some(text_to_string(p.get_host_pseudonym().map_err(|e| capnp_err(&e))?)?)
        } else { None },
    })
}

fn read_stage_update(p: cap::stage_update_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let speakers: Result<Vec<String>, ProtocolError> = p
        .get_speakers()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect();
    Ok(ControlPayload::StageUpdate {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        topic: if p.get_has_topic() {
            Some(text_to_string(p.get_topic().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        speakers: speakers?,
        moderator_pseudonym: text_to_string(p.get_moderator_pseudonym().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_speak_request(p: cap::speak_request_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SpeakRequest {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        requester_pseudonym: text_to_string(p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_speak_response(p: cap::speak_response_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SpeakResponse {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        requester_pseudonym: text_to_string(p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?)?,
        granted: p.get_granted(),
        moderator_pseudonym: text_to_string(p.get_moderator_pseudonym().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_request_attachment(p: cap::request_attachment_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let attachment_id_bytes = p.get_attachment_id().map_err(|e| capnp_err(&e))?;
    let attachment_id: [u8; 16] = attachment_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("attachment_id must be 16 bytes".into()))?;
    let chunks: Vec<u32> = p.get_requested_chunks().map_err(|e| capnp_err(&e))?.iter().collect();
    Ok(ControlPayload::RequestAttachment {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        attachment_id,
        requested_chunks: chunks,
        requester_pseudonym: text_to_string(p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_attachment_chunk(p: cap::attachment_chunk_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let attachment_id_bytes = p.get_attachment_id().map_err(|e| capnp_err(&e))?;
    let attachment_id: [u8; 16] = attachment_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("attachment_id must be 16 bytes".into()))?;
    let plaintext_hash_bytes = p.get_plaintext_hash().map_err(|e| capnp_err(&e))?;
    let plaintext_hash: [u8; 32] = plaintext_hash_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("plaintext_hash must be 32 bytes".into()))?;
    Ok(ControlPayload::AttachmentChunk {
        attachment_id,
        chunk_index: p.get_chunk_index(),
        data: p.get_data().map_err(|e| capnp_err(&e))?.to_vec(),
        plaintext_hash,
    })
}

fn read_multi_attachment_chunk(p: cap::multi_attachment_chunk_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let chunks: Result<Vec<ControlPayload>, ProtocolError> = p
        .get_chunks()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(decode_control_payload)
        .collect();
    Ok(ControlPayload::MultiAttachmentChunk { chunks: chunks? })
}

fn read_voice_mute(p: cap::voice_mute_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceMute {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
        muted: p.get_muted(),
    })
}

fn read_voice_deafen(p: cap::voice_deafen_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceDeafen {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
        deafened: p.get_deafened(),
    })
}

fn read_voice_roster(p: cap::voice_roster_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let participants: Result<Vec<_>, ProtocolError> = p
        .get_participants()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_voice_roster_entry)
        .collect();
    Ok(ControlPayload::VoiceRoster {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        participants: participants?,
    })
}

fn read_soundboard_play(p: cap::soundboard_play_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SoundboardPlay {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        expression_id: text_to_string(p.get_expression_id().map_err(|e| capnp_err(&e))?)?,
        actor_pseudonym: text_to_string(p.get_actor_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_video_fragment(p: cap::video_fragment_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::VideoFragment {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
        frame_seq: p.get_frame_seq(),
        frag_index: p.get_frag_index(),
        frag_total: p.get_frag_total(),
        keyframe: p.get_keyframe(),
        timestamp: p.get_timestamp(),
        payload: p.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
        signature: p.get_signature().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

fn read_video_parity_fragment(p: cap::video_parity_fragment_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::VideoParityFragment {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
        frame_seq: p.get_frame_seq(),
        parity_index: p.get_parity_index(),
        parity_total: p.get_parity_total(),
        data_count: p.get_data_count(),
        frame_len: p.get_frame_len(),
        timestamp: p.get_timestamp(),
        payload: p.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
        signature: p.get_signature().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

fn read_frame_ack(p: cap::frame_ack_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::FrameAck {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
        last_frame_seq: p.get_last_frame_seq(),
        kbps: p.get_kbps(),
        loss_q8: p.get_loss_q8(),
    })
}

fn read_keyframe_request(p: cap::keyframe_request_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::KeyframeRequest {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
    })
}

fn read_bandwidth_estimate(p: cap::bandwidth_estimate_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::BandwidthEstimate {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        kbps: p.get_kbps(),
        window_secs: p.get_window_secs(),
        loss_q8: p.get_loss_q8(),
    })
}

fn read_media_capabilities(p: cap::media_capabilities_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let codecs: Result<Vec<String>, ProtocolError> = p
        .get_codecs()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect();
    Ok(ControlPayload::MediaCapabilities {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        max_pixel_count: p.get_max_pixel_count(),
        max_fps: p.get_max_fps(),
        codecs: codecs?,
    })
}

fn read_topology_change(p: cap::topology_change_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    let stream_id_bytes = p.get_stream_id().map_err(|e| capnp_err(&e))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("stream_id must be 16 bytes".into()))?;
    Ok(ControlPayload::TopologyChange {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        stream_id,
        relay_host_pseudonym: if p.get_has_relay_host() {
            Some(text_to_string(p.get_relay_host_pseudonym().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        reason: text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_link_preview(p: cap::link_preview_payload::Reader<'_>) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::LinkPreview {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        message_id: text_to_string(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        url: text_to_string(p.get_url().map_err(|e| capnp_err(&e))?)?,
        title: if p.get_has_title() {
            Some(text_to_string(p.get_title().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        description: if p.get_has_description() {
            Some(text_to_string(p.get_description().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        image_url: if p.get_has_image_url() {
            Some(text_to_string(p.get_image_url().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        site_name: if p.get_has_site_name() {
            Some(text_to_string(p.get_site_name().map_err(|e| capnp_err(&e))?)?)
        } else { None },
        fetched_at: p.get_fetched_at(),
    })
}
