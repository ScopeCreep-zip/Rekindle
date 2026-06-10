//! `control` voice variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{read_voice_roster_entry, write_voice_roster_entry};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_envelope_capnp as cap;
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;

pub(super) fn write_voice_join(
    mut p: cap::voice_join_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceJoin {
        channel_id,
        route_blob,
        display_name,
    } = payload
    else {
        unreachable!("write_voice_join: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_route_blob(route_blob);
    p.set_display_name(display_name.as_deref().unwrap_or_default());
}

pub(super) fn write_voice_join_ack(
    mut p: cap::voice_join_ack_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceJoinAck {
        channel_id,
        joiner_pseudonym,
        display_name,
        route_blob,
    } = payload
    else {
        unreachable!("write_voice_join_ack: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_joiner_pseudonym(joiner_pseudonym);
    p.set_display_name(display_name.as_deref().unwrap_or_default());
    p.set_route_blob(route_blob);
}

pub(super) fn write_voice_join_confirmed(
    mut p: cap::voice_join_confirmed_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceJoinConfirmed { channel_id } = payload else {
        unreachable!("write_voice_join_confirmed: variant mismatch")
    };
    p.set_channel_id(channel_id);
}

pub(super) fn write_voice_leave(
    mut p: cap::voice_leave_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceLeave { channel_id } = payload else {
        unreachable!("write_voice_leave: variant mismatch")
    };
    p.set_channel_id(channel_id);
}

pub(super) fn write_voice_mode_switch(
    mut p: cap::voice_mode_switch_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceModeSwitch {
        channel_id,
        mode,
        host_pseudonym,
    } = payload
    else {
        unreachable!("write_voice_mode_switch: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_mode(mode);
    p.set_has_host_pseudonym(host_pseudonym.is_some());
    if let Some(h) = host_pseudonym {
        p.set_host_pseudonym(h);
    }
}

pub(super) fn write_stage_update(
    mut p: cap::stage_update_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::StageUpdate {
        channel_id,
        topic,
        speakers,
        moderator_pseudonym,
        lamport,
    } = payload
    else {
        unreachable!("write_stage_update: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_has_topic(topic.is_some());
    if let Some(t) = topic {
        p.set_topic(t);
    }
    let mut list = p.reborrow().init_speakers(len_u32(speakers.len()));
    for (i, s) in speakers.iter().enumerate() {
        list.set(len_u32(i), s.as_str());
    }
    p.set_moderator_pseudonym(moderator_pseudonym);
    p.set_lamport(*lamport);
}

pub(super) fn write_speak_request(
    mut p: cap::speak_request_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::SpeakRequest {
        channel_id,
        requester_pseudonym,
        lamport,
    } = payload
    else {
        unreachable!("write_speak_request: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_lamport(*lamport);
}

pub(super) fn write_speak_response(
    mut p: cap::speak_response_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::SpeakResponse {
        channel_id,
        requester_pseudonym,
        granted,
        moderator_pseudonym,
        lamport,
    } = payload
    else {
        unreachable!("write_speak_response: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_granted(*granted);
    p.set_moderator_pseudonym(moderator_pseudonym);
    p.set_lamport(*lamport);
}

pub(super) fn write_voice_mute(
    mut p: cap::voice_mute_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceMute {
        channel_id,
        target_pseudonym,
        muted,
    } = payload
    else {
        unreachable!("write_voice_mute: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_target_pseudonym(target_pseudonym);
    p.set_muted(*muted);
}

pub(super) fn write_voice_deafen(
    mut p: cap::voice_deafen_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceDeafen {
        channel_id,
        target_pseudonym,
        deafened,
    } = payload
    else {
        unreachable!("write_voice_deafen: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_target_pseudonym(target_pseudonym);
    p.set_deafened(*deafened);
}

pub(super) fn write_voice_roster(
    mut p: cap::voice_roster_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceRoster {
        channel_id,
        participants,
    } = payload
    else {
        unreachable!("write_voice_roster: variant mismatch")
    };
    p.set_channel_id(channel_id);
    let mut list = p.reborrow().init_participants(len_u32(participants.len()));
    for (i, e) in participants.iter().enumerate() {
        write_voice_roster_entry(list.reborrow().get(len_u32(i)), e);
    }
}

pub(super) fn write_soundboard_play(
    mut p: cap::soundboard_play_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::SoundboardPlay {
        channel_id,
        expression_id,
        actor_pseudonym,
    } = payload
    else {
        unreachable!("write_soundboard_play: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_expression_id(expression_id);
    p.set_actor_pseudonym(actor_pseudonym);
}

pub(super) fn read_voice_join(
    p: cap::voice_join_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let display_name = text_to_string(p.get_display_name().map_err(|e| capnp_err(&e))?)?;
    Ok(ControlPayload::VoiceJoin {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        route_blob: p.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec(),
        display_name: (!display_name.is_empty()).then_some(display_name),
    })
}

pub(super) fn read_voice_join_ack(
    p: cap::voice_join_ack_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let display_name = text_to_string(p.get_display_name().map_err(|e| capnp_err(&e))?)?;
    Ok(ControlPayload::VoiceJoinAck {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        joiner_pseudonym: text_to_string(p.get_joiner_pseudonym().map_err(|e| capnp_err(&e))?)?,
        display_name: (!display_name.is_empty()).then_some(display_name),
        route_blob: p.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

pub(super) fn read_voice_join_confirmed(
    p: cap::voice_join_confirmed_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceJoinConfirmed {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_voice_leave(
    p: cap::voice_leave_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceLeave {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_voice_mode_switch(
    p: cap::voice_mode_switch_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceModeSwitch {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        mode: text_to_string(p.get_mode().map_err(|e| capnp_err(&e))?)?,
        host_pseudonym: if p.get_has_host_pseudonym() {
            Some(text_to_string(
                p.get_host_pseudonym().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
    })
}

pub(super) fn read_stage_update(
    p: cap::stage_update_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
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
        } else {
            None
        },
        speakers: speakers?,
        moderator_pseudonym: text_to_string(
            p.get_moderator_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_speak_request(
    p: cap::speak_request_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SpeakRequest {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        requester_pseudonym: text_to_string(
            p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_speak_response(
    p: cap::speak_response_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SpeakResponse {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        requester_pseudonym: text_to_string(
            p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        granted: p.get_granted(),
        moderator_pseudonym: text_to_string(
            p.get_moderator_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_voice_mute(
    p: cap::voice_mute_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceMute {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
        muted: p.get_muted(),
    })
}

pub(super) fn read_voice_deafen(
    p: cap::voice_deafen_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::VoiceDeafen {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        target_pseudonym: text_to_string(p.get_target_pseudonym().map_err(|e| capnp_err(&e))?)?,
        deafened: p.get_deafened(),
    })
}

pub(super) fn read_voice_roster(
    p: cap::voice_roster_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
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

pub(super) fn read_soundboard_play(
    p: cap::soundboard_play_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SoundboardPlay {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        expression_id: text_to_string(p.get_expression_id().map_err(|e| capnp_err(&e))?)?,
        actor_pseudonym: text_to_string(p.get_actor_pseudonym().map_err(|e| capnp_err(&e))?)?,
    })
}
