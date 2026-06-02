//! `control` membership variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{
    read_member_summary, read_onboarding_answer, write_member_summary, write_onboarding_answer,
};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_envelope_capnp as cap;
use crate::dht::community::envelope::ControlPayload;
use crate::error::ProtocolError;

pub(super) fn write_member_join_request(
    mut p: cap::member_join_request_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MemberJoinRequest {
        pseudonym_key,
        display_name,
        invite_code,
        route_blob,
        prekey_bundle,
        claimed_subkey_index,
    } = payload
    else {
        unreachable!("write_member_join_request: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
    p.set_display_name(display_name);
    p.set_has_invite_code(invite_code.is_some());
    if let Some(c) = invite_code {
        p.set_invite_code(c);
    }
    p.set_has_route_blob(route_blob.is_some());
    if let Some(rb) = route_blob {
        p.set_route_blob(rb);
    }
    p.set_has_prekey_bundle(prekey_bundle.is_some());
    if let Some(pb) = prekey_bundle {
        p.set_prekey_bundle(pb);
    }
    p.set_has_claimed_subkey(claimed_subkey_index.is_some());
    if let Some(idx) = claimed_subkey_index {
        p.set_claimed_subkey_index(*idx);
    }
}

pub(super) fn write_member_leave(
    mut p: cap::member_leave_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MemberLeave { pseudonym_key } = payload else {
        unreachable!("write_member_leave: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
}

pub(super) fn write_join_accepted(
    mut p: cap::join_accepted_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::JoinAccepted {
        mek_encrypted,
        mek_generation,
        members,
        member_registry_key,
        slot_index,
        wrapped_slot_seed,
    } = payload
    else {
        unreachable!("write_join_accepted: variant mismatch")
    };
    p.set_mek_encrypted(mek_encrypted);
    p.set_mek_generation(*mek_generation);
    let mut list = p.reborrow().init_members(len_u32(members.len()));
    for (i, m) in members.iter().enumerate() {
        write_member_summary(list.reborrow().get(len_u32(i)), m);
    }
    p.set_has_member_registry_key(member_registry_key.is_some());
    if let Some(k) = member_registry_key {
        p.set_member_registry_key(k);
    }
    p.set_has_slot_index(slot_index.is_some());
    if let Some(idx) = slot_index {
        p.set_slot_index(*idx);
    }
    p.set_has_wrapped_slot_seed(wrapped_slot_seed.is_some());
    if let Some(seed) = wrapped_slot_seed {
        p.set_wrapped_slot_seed(seed);
    }
}

pub(super) fn write_join_rejected(
    mut p: cap::join_rejected_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::JoinRejected { reason } = payload else {
        unreachable!("write_join_rejected: variant mismatch")
    };
    p.set_reason(reason);
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

pub(super) fn write_member_removed(
    mut p: cap::member_removed_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MemberRemoved { pseudonym_key } = payload else {
        unreachable!("write_member_removed: variant mismatch")
    };
    p.set_pseudonym_key(pseudonym_key);
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

pub(super) fn write_member_roles_changed(
    mut p: cap::member_roles_changed_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MemberRolesChanged {
        pseudonym_key,
        role_ids,
    } = payload
    else {
        unreachable!("write_member_roles_changed: variant mismatch")
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

pub(super) fn read_member_join_request(
    p: cap::member_join_request_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MemberJoinRequest {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        display_name: text_to_string(p.get_display_name().map_err(|e| capnp_err(&e))?)?,
        invite_code: if p.get_has_invite_code() {
            Some(text_to_string(
                p.get_invite_code().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        route_blob: if p.get_has_route_blob() {
            Some(p.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec())
        } else {
            None
        },
        prekey_bundle: if p.get_has_prekey_bundle() {
            Some(p.get_prekey_bundle().map_err(|e| capnp_err(&e))?.to_vec())
        } else {
            None
        },
        claimed_subkey_index: if p.get_has_claimed_subkey() {
            Some(p.get_claimed_subkey_index())
        } else {
            None
        },
    })
}

pub(super) fn read_member_leave(
    p: cap::member_leave_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MemberLeave {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_join_accepted(
    p: cap::join_accepted_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
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
            Some(text_to_string(
                p.get_member_registry_key().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        slot_index: if p.get_has_slot_index() {
            Some(p.get_slot_index())
        } else {
            None
        },
        wrapped_slot_seed: if p.get_has_wrapped_slot_seed() {
            Some(
                p.get_wrapped_slot_seed()
                    .map_err(|e| capnp_err(&e))?
                    .to_vec(),
            )
        } else {
            None
        },
    })
}

pub(super) fn read_join_rejected(
    p: cap::join_rejected_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::JoinRejected {
        reason: text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_member_joined(
    p: cap::member_joined_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
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

pub(super) fn read_member_removed(
    p: cap::member_removed_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::MemberRemoved {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_request_segment_expansion(
    p: cap::request_segment_expansion_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
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
) -> Result<ControlPayload, ProtocolError> {
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

pub(super) fn read_member_roles_changed(
    p: cap::member_roles_changed_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let role_ids: Vec<u32> = p
        .get_role_ids()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .collect();
    Ok(ControlPayload::MemberRolesChanged {
        pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        role_ids,
    })
}

pub(super) fn read_channel_overwrite_changed(
    p: cap::channel_overwrite_changed_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::ChannelOverwriteChanged {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_submit_onboarding_answers(
    p: cap::submit_onboarding_answers_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    let answers: Result<Vec<_>, ProtocolError> = p
        .get_answers()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_onboarding_answer)
        .collect();
    Ok(ControlPayload::SubmitOnboardingAnswers { answers: answers? })
}
