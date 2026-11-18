//! `control` keys variant encode/decode helpers.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community::envelope::{ControlPayload, MekTransferAckPayload, MekTransferPayload};
use crate::community_envelope_capnp as cap;
use crate::error::CodecError;

pub(super) fn write_mek_rotated(
    mut p: cap::m_e_k_rotated_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MEKRotated {
        channel_id,
        new_generation,
        rotator_pseudonym,
    } = payload
    else {
        unreachable!("write_mek_rotated: variant mismatch")
    };
    p.set_has_channel_id(channel_id.is_some());
    if let Some(c) = channel_id {
        p.set_channel_id(c);
    }
    p.set_new_generation(*new_generation);
    p.set_has_rotator_pseudonym(rotator_pseudonym.is_some());
    if let Some(rp) = rotator_pseudonym {
        p.set_rotator_pseudonym(rp);
    }
}

pub(super) fn write_request_mek(
    mut p: cap::request_m_e_k_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::RequestMEK {
        channel_id,
        needed_generation,
        requester_pseudonym,
        cascade_index,
    } = payload
    else {
        unreachable!("write_request_mek: variant mismatch")
    };
    p.set_channel_id(channel_id);
    p.set_needed_generation(*needed_generation);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_cascade_index(*cascade_index);
}

pub(super) fn write_mek_transfer(
    mut p: cap::mek_transfer_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MekTransfer(MekTransferPayload {
        community_id,
        channel_id,
        generation,
        sender_pseudonym,
        wrapped_mek,
        transfer_id,
    }) = payload
    else {
        unreachable!("write_mek_transfer: variant mismatch")
    };
    p.set_community_id(community_id);
    p.set_has_channel_id(channel_id.is_some());
    if let Some(c) = channel_id {
        p.set_channel_id(c);
    }
    p.set_generation(*generation);
    p.set_sender_pseudonym(sender_pseudonym);
    p.set_wrapped_mek(wrapped_mek);
    p.set_transfer_id(transfer_id);
}

pub(super) fn write_mek_transfer_ack(
    mut p: cap::mek_transfer_ack_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::MekTransferAck(MekTransferAckPayload {
        community_id,
        channel_id,
        generation,
        requester_pseudonym,
        transfer_id,
        ack_tag,
    }) = payload
    else {
        unreachable!("write_mek_transfer_ack: variant mismatch")
    };
    p.set_community_id(community_id);
    p.set_has_channel_id(channel_id.is_some());
    if let Some(c) = channel_id {
        p.set_channel_id(c);
    }
    p.set_generation(*generation);
    p.set_requester_pseudonym(requester_pseudonym);
    p.set_transfer_id(transfer_id);
    p.set_ack_tag(ack_tag);
}

pub(super) fn read_mek_rotated(
    p: cap::m_e_k_rotated_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::MEKRotated {
        channel_id: if p.get_has_channel_id() {
            Some(text_to_string(
                p.get_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        new_generation: p.get_new_generation(),
        rotator_pseudonym: if p.get_has_rotator_pseudonym() {
            Some(text_to_string(
                p.get_rotator_pseudonym().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
    })
}

pub(super) fn read_request_mek(
    p: cap::request_m_e_k_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::RequestMEK {
        channel_id: text_to_string(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        needed_generation: p.get_needed_generation(),
        requester_pseudonym: text_to_string(
            p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        // Cap'n Proto fills missing fields with zero on read, so peers
        // pre-dating cascadeIndex transparently produce cascade_index=0
        // (the deterministic top-rank responder).
        cascade_index: p.get_cascade_index(),
    })
}

pub(super) fn read_mek_transfer(
    p: cap::mek_transfer_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::MekTransfer(MekTransferPayload {
        community_id: text_to_string(p.get_community_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: if p.get_has_channel_id() {
            Some(text_to_string(
                p.get_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        generation: p.get_generation(),
        sender_pseudonym: text_to_string(p.get_sender_pseudonym().map_err(|e| capnp_err(&e))?)?,
        wrapped_mek: p.get_wrapped_mek().map_err(|e| capnp_err(&e))?.to_vec(),
        transfer_id: p.get_transfer_id().map_err(|e| capnp_err(&e))?.to_vec(),
    }))
}

pub(super) fn read_mek_transfer_ack(
    p: cap::mek_transfer_ack_payload::Reader<'_>,
) -> Result<ControlPayload, CodecError> {
    Ok(ControlPayload::MekTransferAck(MekTransferAckPayload {
        community_id: text_to_string(p.get_community_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: if p.get_has_channel_id() {
            Some(text_to_string(
                p.get_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        generation: p.get_generation(),
        requester_pseudonym: text_to_string(
            p.get_requester_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        transfer_id: p.get_transfer_id().map_err(|e| capnp_err(&e))?.to_vec(),
        ack_tag: p.get_ack_tag().map_err(|e| capnp_err(&e))?.to_vec(),
    }))
}
