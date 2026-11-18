//! `control` membership variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{read_onboarding_answer, write_onboarding_answer};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community::envelope::ControlPayload;
use crate::community_envelope_capnp as cap;
use crate::error::CodecError;

pub(super) fn write_member_leave(
    mut p: cap::member_leave_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MemberLeave { pseudonym_key } = payload else {
        unreachable!("write_member_leave: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
}

pub(super) fn write_member_joined(
    mut p: cap::member_joined_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MemberJoined {
        pseudonym_key,
        display_name,
        role_ids,
        status,
        route_blob,
    } = payload
    else {
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
    if let Some(rb) = route_blob {
        p.set_route_blob(rb);
    }
}

pub(super) fn write_request_segment_expansion(
    mut p: cap::request_segment_expansion_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::RequestSegmentExpansion {
        community_id,
        requester_pseudonym,
        full_segment_index,
    } = payload
    else {
        unreachable!("write_request_segment_expansion: variant mismatch")
    };
    p.set_community_id(community_id);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_full_segment_index(*full_segment_index);
}

pub(super) fn write_onboarding_complete(
    mut p: cap::onboarding_complete_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::OnboardingComplete {
        pseudonym_key,
        role_ids,
    } = payload
    else {
        unreachable!("write_onboarding_complete: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
    let mut list = p.reborrow().init_role_ids(len_u32(role_ids.len()));
    for (i, id) in role_ids.iter().enumerate() {
        list.set(len_u32(i), *id);
    }
}

pub(super) fn write_channel_overwrite_changed(
    mut p: cap::channel_overwrite_changed_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::ChannelOverwriteChanged { channel_id } = payload else {
        unreachable!("write_channel_overwrite_changed: variant mismatch")
    };
    p.set_channel_id(channel_id);
}

pub(super) fn write_submit_onboarding_answers(
    mut p: cap::submit_onboarding_answers_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::SubmitOnboardingAnswers { answers } = payload else {
        unreachable!("write_submit_onboarding_answers: variant mismatch")
    };
    let mut list = p.reborrow().init_answers(len_u32(answers.len()));
    for (i, a) in answers.iter().enumerate() {
        write_onboarding_answer(list.reborrow().get(len_u32(i)), a);
    }
}

pub(super) fn read_member_leave(
    p: cap::member_leave_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::MemberLeave {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_member_joined(
    p: cap::member_joined_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    let role_ids: Vec<u32> = p
        .get_role_ids()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .collect();
    Ok(ControlPayload::MemberJoined {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        display_name: text_to_string(p.get_display_name().map_err(|e| capnp_err(&e))?)?,
        role_ids,
        status: text_to_string(p.get_status().map_err(|e| capnp_err(&e))?)?,
        route_blob: if p.get_has_route_blob() {
            Some(p.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec())
        } else {
            None
        },
    })
}

pub(super) fn read_request_segment_expansion(
    p: cap::request_segment_expansion_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::RequestSegmentExpansion {
        community_id: text_to_string(p.get_community_id().map_err(|e| capnp_err(&e))?)?,
        requester_pseudonym: text_to_string(
            p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        full_segment_index: p.get_full_segment_index(),
    })
}

pub(super) fn read_onboarding_complete(
    p: cap::onboarding_complete_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    let role_ids: Vec<u32> = p
        .get_role_ids()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .collect();
    Ok(ControlPayload::OnboardingComplete {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        role_ids,
    })
}

pub(super) fn read_channel_overwrite_changed(
    p: cap::channel_overwrite_changed_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::ChannelOverwriteChanged {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_submit_onboarding_answers(
    p: cap::submit_onboarding_answers_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    let answers: Result<Vec<_>, CodecError> = p
        .get_answers()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_onboarding_answer)
        .collect();
    Ok(ControlPayload::SubmitOnboardingAnswers { answers: answers? })
}
