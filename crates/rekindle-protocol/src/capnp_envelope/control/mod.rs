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

use crate::capnp_codec::{capnp_err, not_in_schema};
use crate::community_envelope_capnp::control_payload as schema;
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;

mod bootstrap_sync;
mod events;
mod keys;
mod media;
mod membership;
mod messaging;
mod moderation;
mod voice;

use bootstrap_sync::{
    read_bootstrap_request, read_bootstrap_response, read_sync_request, read_sync_response,
    write_bootstrap_request, write_bootstrap_response, write_sync_request, write_sync_response,
};
use events::{
    read_event_created, read_event_deleted, read_event_reminder, read_event_rsvp_changed,
    read_event_updated, read_game_server_added, read_game_server_removed, read_thread_archived,
    read_thread_created, read_thread_message_received, write_event_created, write_event_deleted,
    write_event_reminder, write_event_rsvp_changed, write_event_updated, write_game_server_added,
    write_game_server_removed, write_thread_archived, write_thread_created,
    write_thread_message_received,
};
use keys::{
    read_admin_keypair_grant, read_governance_updated, read_mek_rotated, read_mek_transfer,
    read_mek_transfer_ack, read_request_mek, read_slot_keypair_grant, write_admin_keypair_grant,
    write_governance_updated, write_mek_rotated, write_mek_transfer, write_mek_transfer_ack,
    write_request_mek, write_slot_keypair_grant,
};
use media::{
    read_attachment_chunk, read_bandwidth_estimate, read_frame_ack, read_keyframe_request,
    read_media_capabilities, read_multi_attachment_chunk, read_request_attachment,
    read_topology_change, read_video_fragment, read_video_parity_fragment, write_attachment_chunk,
    write_bandwidth_estimate, write_frame_ack, write_keyframe_request, write_media_capabilities,
    write_multi_attachment_chunk, write_request_attachment, write_topology_change,
    write_video_fragment, write_video_parity_fragment,
};
use membership::{
    read_channel_overwrite_changed, read_join_accepted, read_join_rejected,
    read_member_join_request, read_member_joined, read_member_leave, read_member_removed,
    read_member_roles_changed, read_onboarding_complete, read_request_segment_expansion,
    read_submit_onboarding_answers, write_channel_overwrite_changed, write_join_accepted,
    write_join_rejected, write_member_join_request, write_member_joined, write_member_leave,
    write_member_removed, write_member_roles_changed, write_onboarding_complete,
    write_request_segment_expansion, write_submit_onboarding_answers,
};
use messaging::{
    read_link_preview, read_message_deleted, read_message_edited, read_message_pinned,
    read_message_unpinned, read_reaction_added, read_reaction_removed, write_link_preview,
    write_message_deleted, write_message_edited, write_message_pinned, write_message_unpinned,
    write_reaction_added, write_reaction_removed,
};
use moderation::{
    read_ban, read_channel_lockdown, read_kick, read_member_timed_out, read_raid_alert,
    read_remove_timeout, read_system_message, read_timeout_member, read_unban, write_ban,
    write_channel_lockdown, write_kick, write_member_timed_out, write_raid_alert,
    write_remove_timeout, write_system_message, write_timeout_member, write_unban,
};
use voice::{
    read_soundboard_play, read_speak_request, read_speak_response, read_stage_update,
    read_voice_deafen, read_voice_join, read_voice_leave, read_voice_mode_switch, read_voice_mute,
    read_voice_roster, write_soundboard_play, write_speak_request, write_speak_response,
    write_stage_update, write_voice_deafen, write_voice_join, write_voice_leave,
    write_voice_mode_switch, write_voice_mute, write_voice_roster,
};

pub fn encode_control_payload(
    b: schema::Builder<'_>,
    payload: &ControlPayload,
) -> Result<(), ProtocolError> {
    let mut b = b;
    use ControlPayload as CP;
    match payload {
        CP::MemberJoinRequest { .. } => {
            write_member_join_request(b.reborrow().init_member_join_request(), payload);
        }
        CP::MemberLeave { .. } => write_member_leave(b.reborrow().init_member_leave(), payload),
        CP::JoinAccepted { .. } => write_join_accepted(b.reborrow().init_join_accepted(), payload),
        CP::JoinRejected { .. } => write_join_rejected(b.reborrow().init_join_rejected(), payload),
        CP::MemberJoined { .. } => write_member_joined(b.reborrow().init_member_joined(), payload),
        CP::MemberRemoved { .. } => {
            write_member_removed(b.reborrow().init_member_removed(), payload);
        }
        CP::Kick { .. } => write_kick(b.reborrow().init_kick(), payload),
        CP::Ban { .. } => write_ban(b.reborrow().init_ban(), payload),
        CP::Unban { .. } => write_unban(b.reborrow().init_unban(), payload),
        CP::TimeoutMember { .. } => {
            write_timeout_member(b.reborrow().init_timeout_member(), payload);
        }
        CP::RemoveTimeout { .. } => {
            write_remove_timeout(b.reborrow().init_remove_timeout(), payload);
        }
        CP::MemberTimedOut { .. } => {
            write_member_timed_out(b.reborrow().init_member_timed_out(), payload);
        }
        CP::MessageEdited { .. } => {
            write_message_edited(b.reborrow().init_message_edited(), payload);
        }
        CP::MessageDeleted { .. } => {
            write_message_deleted(b.reborrow().init_message_deleted(), payload);
        }
        CP::MEKRotated { .. } => write_mek_rotated(b.reborrow().init_mek_rotated(), payload),
        CP::RequestMEK { .. } => write_request_mek(b.reborrow().init_request_mek(), payload),
        CP::MekTransfer { .. } => write_mek_transfer(b.reborrow().init_mek_transfer(), payload),
        CP::MekTransferAck { .. } => {
            write_mek_transfer_ack(b.reborrow().init_mek_transfer_ack(), payload);
        }
        CP::RequestSegmentExpansion { .. } => {
            write_request_segment_expansion(b.reborrow().init_request_segment_expansion(), payload);
        }
        CP::OnboardingComplete { .. } => {
            write_onboarding_complete(b.reborrow().init_onboarding_complete(), payload);
        }
        CP::MemberRolesChanged { .. } => {
            write_member_roles_changed(b.reborrow().init_member_roles_changed(), payload);
        }
        CP::ChannelOverwriteChanged { .. } => {
            write_channel_overwrite_changed(b.reborrow().init_channel_overwrite_changed(), payload);
        }
        CP::ReactionAdded { .. } => {
            write_reaction_added(b.reborrow().init_reaction_added(), payload);
        }
        CP::ReactionRemoved { .. } => {
            write_reaction_removed(b.reborrow().init_reaction_removed(), payload);
        }
        CP::MessagePinned { .. } => {
            write_message_pinned(b.reborrow().init_message_pinned(), payload);
        }
        CP::MessageUnpinned { .. } => {
            write_message_unpinned(b.reborrow().init_message_unpinned(), payload);
        }
        CP::EventCreated { .. } => write_event_created(b.reborrow().init_event_created(), payload),
        CP::EventUpdated { .. } => write_event_updated(b.reborrow().init_event_updated(), payload),
        CP::EventDeleted { .. } => write_event_deleted(b.reborrow().init_event_deleted(), payload),
        CP::EventRsvpChanged { .. } => {
            write_event_rsvp_changed(b.reborrow().init_event_rsvp_changed(), payload);
        }
        CP::ThreadCreated { .. } => {
            write_thread_created(b.reborrow().init_thread_created(), payload);
        }
        CP::ThreadMessageReceived { .. } => {
            write_thread_message_received(b.reborrow().init_thread_message_received(), payload);
        }
        CP::ThreadArchived { .. } => {
            write_thread_archived(b.reborrow().init_thread_archived(), payload);
        }
        CP::GameServerAdded { .. } => {
            write_game_server_added(b.reborrow().init_game_server_added(), payload);
        }
        CP::GameServerRemoved { .. } => {
            write_game_server_removed(b.reborrow().init_game_server_removed(), payload);
        }
        CP::SubmitOnboardingAnswers { .. } => {
            write_submit_onboarding_answers(b.reborrow().init_submit_onboarding_answers(), payload);
        }
        CP::EventReminder { .. } => {
            write_event_reminder(b.reborrow().init_event_reminder(), payload);
        }
        CP::KickedNotification => b.reborrow().set_kicked_notification(()),
        CP::RaidAlert { .. } => write_raid_alert(b.reborrow().init_raid_alert(), payload),
        CP::ChannelLockdown { .. } => {
            write_channel_lockdown(b.reborrow().init_channel_lockdown(), payload);
        }
        CP::SystemMessage { .. } => {
            write_system_message(b.reborrow().init_system_message(), payload);
        }
        CP::AdminKeypairGrant { .. } => {
            write_admin_keypair_grant(b.reborrow().init_admin_keypair_grant(), payload);
        }
        CP::SlotKeypairGrant { .. } => {
            write_slot_keypair_grant(b.reborrow().init_slot_keypair_grant(), payload);
        }
        CP::GovernanceUpdated { .. } => {
            write_governance_updated(b.reborrow().init_governance_updated(), payload);
        }
        CP::BootstrapRequest { .. } => {
            write_bootstrap_request(b.reborrow().init_bootstrap_request(), payload);
        }
        CP::BootstrapResponse { .. } => {
            write_bootstrap_response(b.reborrow().init_bootstrap_response(), payload);
        }
        CP::SyncRequest { .. } => write_sync_request(b.reborrow().init_sync_request(), payload),
        CP::SyncResponse { .. } => write_sync_response(b.reborrow().init_sync_response(), payload),
        CP::VoiceJoin { .. } => write_voice_join(b.reborrow().init_voice_join(), payload),
        CP::VoiceLeave { .. } => write_voice_leave(b.reborrow().init_voice_leave(), payload),
        CP::VoiceModeSwitch { .. } => {
            write_voice_mode_switch(b.reborrow().init_voice_mode_switch(), payload);
        }
        CP::StageUpdate { .. } => write_stage_update(b.reborrow().init_stage_update(), payload),
        CP::SpeakRequest { .. } => write_speak_request(b.reborrow().init_speak_request(), payload),
        CP::SpeakResponse { .. } => {
            write_speak_response(b.reborrow().init_speak_response(), payload);
        }
        CP::RequestAttachment { .. } => {
            write_request_attachment(b.reborrow().init_request_attachment(), payload);
        }
        CP::AttachmentChunk { .. } => {
            write_attachment_chunk(b.reborrow().init_attachment_chunk(), payload);
        }
        CP::MultiAttachmentChunk { .. } => {
            write_multi_attachment_chunk(b.reborrow().init_multi_attachment_chunk(), payload)?;
        }
        CP::VoiceMute { .. } => write_voice_mute(b.reborrow().init_voice_mute(), payload),
        CP::VoiceDeafen { .. } => write_voice_deafen(b.reborrow().init_voice_deafen(), payload),
        CP::VoiceRoster { .. } => write_voice_roster(b.reborrow().init_voice_roster(), payload),
        CP::SoundboardPlay { .. } => {
            write_soundboard_play(b.reborrow().init_soundboard_play(), payload);
        }
        CP::VideoFragment { .. } => {
            write_video_fragment(b.reborrow().init_video_fragment(), payload);
        }
        CP::VideoParityFragment { .. } => {
            write_video_parity_fragment(b.reborrow().init_video_parity_fragment(), payload);
        }
        CP::FrameAck { .. } => write_frame_ack(b.reborrow().init_frame_ack(), payload),
        CP::KeyframeRequest { .. } => {
            write_keyframe_request(b.reborrow().init_keyframe_request(), payload);
        }
        CP::BandwidthEstimate { .. } => {
            write_bandwidth_estimate(b.reborrow().init_bandwidth_estimate(), payload);
        }
        CP::MediaCapabilities { .. } => {
            write_media_capabilities(b.reborrow().init_media_capabilities(), payload);
        }
        CP::TopologyChange { .. } => {
            write_topology_change(b.reborrow().init_topology_change(), payload);
        }
        CP::LinkPreview { .. } => write_link_preview(b.reborrow().init_link_preview(), payload),
    }
    Ok(())
}

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
        Which::RequestSegmentExpansion(p) => {
            read_request_segment_expansion(p.map_err(|e| capnp_err(&e))?)
        }
        Which::OnboardingComplete(p) => read_onboarding_complete(p.map_err(|e| capnp_err(&e))?),
        Which::MemberRolesChanged(p) => read_member_roles_changed(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelOverwriteChanged(p) => {
            read_channel_overwrite_changed(p.map_err(|e| capnp_err(&e))?)
        }
        Which::ReactionAdded(p) => read_reaction_added(p.map_err(|e| capnp_err(&e))?),
        Which::ReactionRemoved(p) => read_reaction_removed(p.map_err(|e| capnp_err(&e))?),
        Which::MessagePinned(p) => read_message_pinned(p.map_err(|e| capnp_err(&e))?),
        Which::MessageUnpinned(p) => read_message_unpinned(p.map_err(|e| capnp_err(&e))?),
        Which::EventCreated(p) => read_event_created(p.map_err(|e| capnp_err(&e))?),
        Which::EventUpdated(p) => read_event_updated(p.map_err(|e| capnp_err(&e))?),
        Which::EventDeleted(p) => read_event_deleted(p.map_err(|e| capnp_err(&e))?),
        Which::EventRsvpChanged(p) => read_event_rsvp_changed(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadCreated(p) => read_thread_created(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadMessageReceived(p) => {
            read_thread_message_received(p.map_err(|e| capnp_err(&e))?)
        }
        Which::ThreadArchived(p) => read_thread_archived(p.map_err(|e| capnp_err(&e))?),
        Which::GameServerAdded(p) => read_game_server_added(p.map_err(|e| capnp_err(&e))?),
        Which::GameServerRemoved(p) => read_game_server_removed(p.map_err(|e| capnp_err(&e))?),
        Which::SubmitOnboardingAnswers(p) => {
            read_submit_onboarding_answers(p.map_err(|e| capnp_err(&e))?)
        }
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
        Which::MultiAttachmentChunk(p) => {
            read_multi_attachment_chunk(p.map_err(|e| capnp_err(&e))?)
        }
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
