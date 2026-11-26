//! `ControlPayload` encoder / decoder.
//!
//! Mirrors the Rust enum in `crate::community::envelope::ControlPayload`
//! against the schema in `schemas/community_envelope.capnp`. Each
//! variant has its own `write_<name>` / `read_<name>` helper so the
//! dispatchers stay short and individual variant logic is independently
//! reviewable. Helpers take the typed sub-builder/reader plus a
//! `&ControlPayload` and use `let ... else { unreachable!() }` to
//! re-extract the variant's fields. The dispatcher only routes by
//! discriminant.

use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community::envelope::ControlPayload;
use crate::community_envelope_capnp::control_payload as schema;
use crate::error::CodecError;

mod bootstrap_sync;
mod events;
mod keys;
mod media;
mod membership;
mod messaging;
mod moderation;
mod voice;

use bootstrap_sync::{
    read_bootstrap_request, read_bootstrap_response_v2, write_bootstrap_request,
    write_bootstrap_response_v2,
};
use events::{
    read_event_created, read_event_deleted, read_event_rsvp_changed, read_event_updated,
    read_game_server_added, read_game_server_removed, read_thread_message_received,
    write_event_created, write_event_deleted, write_event_rsvp_changed, write_event_updated,
    write_game_server_added, write_game_server_removed, write_thread_message_received,
};
use keys::{
    read_mek_rotated, read_mek_transfer, read_mek_transfer_ack, read_request_mek,
    write_mek_rotated, write_mek_transfer, write_mek_transfer_ack, write_request_mek,
};
use media::{
    read_attachment_chunk, read_keyframe_request, read_media_capabilities,
    read_multi_attachment_chunk, read_request_attachment, read_topology_change,
    read_video_fragment, read_video_parity_fragment, write_attachment_chunk,
    write_keyframe_request, write_media_capabilities, write_multi_attachment_chunk,
    write_request_attachment, write_topology_change, write_video_fragment,
    write_video_parity_fragment,
};
use membership::{
    read_member_joined, read_member_leave, read_onboarding_complete,
    read_request_segment_expansion, read_submit_onboarding_answers, write_member_joined,
    write_member_leave, write_onboarding_complete, write_request_segment_expansion,
    write_submit_onboarding_answers,
};
use messaging::{
    read_link_preview, read_message_pinned, read_message_unpinned, read_reaction_added,
    read_reaction_removed, write_link_preview, write_message_pinned, write_message_unpinned,
    write_reaction_added, write_reaction_removed,
};
use moderation::{read_kick, write_kick};
use voice::{
    read_soundboard_play, read_speak_request, read_speak_response, read_stage_update,
    read_voice_deafen, read_voice_join, read_voice_join_ack, read_voice_join_confirmed,
    read_voice_leave, read_voice_media_key, read_voice_media_key_ack, read_voice_media_key_request,
    read_voice_mode_switch, read_voice_mute, read_voice_roster, write_soundboard_play,
    write_speak_request, write_speak_response, write_stage_update, write_voice_deafen,
    write_voice_join, write_voice_join_ack, write_voice_join_confirmed, write_voice_leave,
    write_voice_media_key, write_voice_media_key_ack, write_voice_media_key_request,
    write_voice_mode_switch, write_voice_mute, write_voice_roster,
};

pub fn encode_control_payload(
    b: schema::Builder<'_>,
    payload: &ControlPayload,
) -> Result<(), CodecError> {
    let mut b = b;
    use ControlPayload as CP;
    match payload {
        CP::MemberLeave { .. } => write_member_leave(b.reborrow().init_member_leave(), payload),
        CP::MemberJoined { .. } => write_member_joined(b.reborrow().init_member_joined(), payload),
        CP::Kick { .. } => write_kick(b.reborrow().init_kick(), payload),
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
        CP::ThreadMessageReceived { .. } => {
            write_thread_message_received(b.reborrow().init_thread_message_received(), payload);
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
        CP::BootstrapRequest { .. } => {
            write_bootstrap_request(b.reborrow().init_bootstrap_request(), payload);
        }
        CP::BootstrapResponseV2 { .. } => {
            write_bootstrap_response_v2(b.reborrow().init_bootstrap_response_v2(), payload)?;
        }
        CP::VoiceJoin { .. } => write_voice_join(b.reborrow().init_voice_join(), payload),
        CP::VoiceJoinAck { .. } => {
            write_voice_join_ack(b.reborrow().init_voice_join_ack(), payload);
        }
        CP::VoiceJoinConfirmed { .. } => {
            write_voice_join_confirmed(b.reborrow().init_voice_join_confirmed(), payload);
        }
        CP::VoiceMediaKey { .. } => {
            write_voice_media_key(b.reborrow().init_voice_media_key(), payload);
        }
        CP::GovernanceOp { op } => b.set_governance_op(op),
        CP::HeadAdvanced { hint } => b.set_head_advanced(hint),
        CP::CheckpointStateRequest { request } => b.set_checkpoint_state_request(request),
        CP::CheckpointStateResponse { response } => b.set_checkpoint_state_response(response),
        CP::CheckpointHintRequest { request } => b.set_checkpoint_hint_request(request),
        CP::CheckpointHintResponse { response } => b.set_checkpoint_hint_response(response),
        CP::FrontierRequest { request } => b.set_frontier_request(request),
        CP::FrontierResponse { frontier } => b.set_frontier_response(frontier),
        CP::BootstrapPartial { anchor } => b.set_bootstrap_partial(anchor),
        CP::BootstrapRefused { anchor, reason } => {
            let mut r = b.reborrow().init_bootstrap_refused();
            r.set_anchor(anchor);
            r.set_reason(reason.to_wire());
        }
        CP::VoiceMediaKeyAck { .. } => {
            write_voice_media_key_ack(b.reborrow().init_voice_media_key_ack(), payload);
        }
        CP::VoiceMediaKeyRequest { .. } => {
            write_voice_media_key_request(b.reborrow().init_voice_media_key_request(), payload);
        }
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
        CP::KeyframeRequest { .. } => {
            write_keyframe_request(b.reborrow().init_keyframe_request(), payload);
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

pub fn decode_control_payload(r: schema::Reader<'_>) -> Result<ControlPayload, CodecError> {
    use schema::Which;
    match r.which().map_err(not_in_schema)? {
        Which::MemberLeave(p) => read_member_leave(p.map_err(|e| capnp_err(&e))?),
        Which::MemberJoined(p) => read_member_joined(p.map_err(|e| capnp_err(&e))?),
        Which::Kick(p) => read_kick(p.map_err(|e| capnp_err(&e))?),
        Which::MekRotated(p) => read_mek_rotated(p.map_err(|e| capnp_err(&e))?),
        Which::RequestMek(p) => read_request_mek(p.map_err(|e| capnp_err(&e))?),
        Which::MekTransfer(p) => read_mek_transfer(p.map_err(|e| capnp_err(&e))?),
        Which::MekTransferAck(p) => read_mek_transfer_ack(p.map_err(|e| capnp_err(&e))?),
        Which::RequestSegmentExpansion(p) => {
            read_request_segment_expansion(p.map_err(|e| capnp_err(&e))?)
        }
        Which::OnboardingComplete(p) => read_onboarding_complete(p.map_err(|e| capnp_err(&e))?),
        Which::ReactionAdded(p) => read_reaction_added(p.map_err(|e| capnp_err(&e))?),
        Which::ReactionRemoved(p) => read_reaction_removed(p.map_err(|e| capnp_err(&e))?),
        Which::MessagePinned(p) => read_message_pinned(p.map_err(|e| capnp_err(&e))?),
        Which::MessageUnpinned(p) => read_message_unpinned(p.map_err(|e| capnp_err(&e))?),
        Which::EventCreated(p) => read_event_created(p.map_err(|e| capnp_err(&e))?),
        Which::EventUpdated(p) => read_event_updated(p.map_err(|e| capnp_err(&e))?),
        Which::EventDeleted(p) => read_event_deleted(p.map_err(|e| capnp_err(&e))?),
        Which::EventRsvpChanged(p) => read_event_rsvp_changed(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadMessageReceived(p) => {
            read_thread_message_received(p.map_err(|e| capnp_err(&e))?)
        }
        Which::GameServerAdded(p) => read_game_server_added(p.map_err(|e| capnp_err(&e))?),
        Which::GameServerRemoved(p) => read_game_server_removed(p.map_err(|e| capnp_err(&e))?),
        Which::SubmitOnboardingAnswers(p) => {
            read_submit_onboarding_answers(p.map_err(|e| capnp_err(&e))?)
        }
        Which::BootstrapRequest(p) => read_bootstrap_request(p.map_err(|e| capnp_err(&e))?),
        Which::BootstrapResponseV2(p) => read_bootstrap_response_v2(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceJoin(p) => read_voice_join(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceJoinAck(p) => read_voice_join_ack(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceJoinConfirmed(p) => read_voice_join_confirmed(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceMediaKey(p) => read_voice_media_key(p.map_err(|e| capnp_err(&e))?),
        Which::VoiceMediaKeyAck(p) => read_voice_media_key_ack(p.map_err(|e| capnp_err(&e))?),
        Which::HeadAdvanced(hint) => Ok(ControlPayload::HeadAdvanced {
            hint: hint.map_err(|e| capnp_err(&e))?.to_vec(),
        }),
        Which::CheckpointStateRequest(request) => Ok(ControlPayload::CheckpointStateRequest {
            request: request.map_err(|e| capnp_err(&e))?.to_vec(),
        }),
        Which::CheckpointStateResponse(response) => Ok(ControlPayload::CheckpointStateResponse {
            response: response.map_err(|e| capnp_err(&e))?.to_vec(),
        }),
        Which::CheckpointHintRequest(request) => Ok(ControlPayload::CheckpointHintRequest {
            request: request.map_err(|e| capnp_err(&e))?.to_vec(),
        }),
        Which::CheckpointHintResponse(response) => Ok(ControlPayload::CheckpointHintResponse {
            response: response.map_err(|e| capnp_err(&e))?.to_vec(),
        }),
        Which::FrontierRequest(request) => Ok(ControlPayload::FrontierRequest {
            request: request.map_err(|e| capnp_err(&e))?.to_vec(),
        }),
        Which::FrontierResponse(frontier) => Ok(ControlPayload::FrontierResponse {
            frontier: frontier.map_err(|e| capnp_err(&e))?.to_vec(),
        }),
        Which::BootstrapPartial(anchor) => Ok(ControlPayload::BootstrapPartial {
            anchor: text_to_string(anchor.map_err(|e| capnp_err(&e))?)?,
        }),
        Which::BootstrapRefused(r) => {
            let r = r.map_err(|e| capnp_err(&e))?;
            Ok(ControlPayload::BootstrapRefused {
                anchor: text_to_string(r.get_anchor().map_err(|e| capnp_err(&e))?)?,
                reason: rekindle_types::join_refusal::JoinRefusal::from_wire(r.get_reason()),
            })
        }
        Which::GovernanceOp(op) => Ok(ControlPayload::GovernanceOp {
            op: op.map_err(|e| capnp_err(&e))?.to_vec(),
        }),
        Which::VoiceMediaKeyRequest(p) => {
            read_voice_media_key_request(p.map_err(|e| capnp_err(&e))?)
        }
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
        Which::KeyframeRequest(p) => read_keyframe_request(p.map_err(|e| capnp_err(&e))?),
        // Removed variants (plan E4.3.3; @35 by E3.2 S9 own removal; the
        // unsigned membership notices, the legacy join answer, the keypair
        // grants and `governanceUpdated` by E3.2 S8, replaced by signed
        // governance ops; the senderless overwrite, thread, reminder, raid,
        // lockdown and system notices by E3.2 S8-E): skipped like an
        // unknown one.
        Which::Obsolete0(())
        | Which::Obsolete2(())
        | Which::Obsolete3(())
        | Which::Obsolete5(())
        | Which::Obsolete7(())
        | Which::Obsolete8(())
        | Which::Obsolete9(())
        | Which::Obsolete10(())
        | Which::Obsolete11(())
        | Which::Obsolete12(())
        | Which::Obsolete13(())
        | Which::Obsolete18(())
        | Which::Obsolete19(())
        | Which::Obsolete28(())
        | Which::Obsolete30(())
        | Which::Obsolete34(())
        | Which::Obsolete35(())
        | Which::Obsolete36(())
        | Which::Obsolete37(())
        | Which::Obsolete38(())
        | Which::Obsolete39(())
        | Which::Obsolete40(())
        | Which::Obsolete41(())
        | Which::Obsolete43(())
        | Which::Obsolete44(())
        | Which::Obsolete45(())
        | Which::Obsolete61(())
        | Which::Obsolete63(()) => Err(CodecError::UnknownVariant(
            "obsolete control payload".to_string(),
        )),
        Which::MediaCapabilities(p) => read_media_capabilities(p.map_err(|e| capnp_err(&e))?),
        Which::TopologyChange(p) => read_topology_change(p.map_err(|e| capnp_err(&e))?),
        Which::LinkPreview(p) => read_link_preview(p.map_err(|e| capnp_err(&e))?),
    }
}
