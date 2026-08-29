//! `control` keys variant encode/decode helpers.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_envelope_capnp as cap;
use crate::dht::community::envelope::{ControlPayload, MekTransferAckPayload, MekTransferPayload};
use crate::error::ProtocolError;

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
}

pub(super) fn write_admin_keypair_grant(
    mut p: cap::admin_keypair_grant_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::AdminKeypairGrant {
        wrapped_owner_keypair,
        wrapped_slot_seed,
    } = payload
    else {
        unreachable!("write_admin_keypair_grant: variant mismatch")
    };
    p.set_wrapped_owner_keypair(wrapped_owner_keypair);
    p.set_wrapped_slot_seed(wrapped_slot_seed);
}

pub(super) fn write_slot_keypair_grant(
    mut p: cap::slot_keypair_grant_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::SlotKeypairGrant {
        slot_index,
        segment_index,
        wrapped_slot_keypair,
    } = payload
    else {
        unreachable!("write_slot_keypair_grant: variant mismatch")
    };
    p.set_slot_index(*slot_index);
    p.set_segment_index(*segment_index);
    p.set_wrapped_slot_keypair(wrapped_slot_keypair);
}

pub(super) fn write_governance_updated(
    mut p: cap::governance_updated_payload::Builder<'_>,
    payload: &ControlPayload,
) {
    let ControlPayload::GovernanceUpdated {
        governance_key,
        subkey_index,
        lamport_ts,
    } = payload
    else {
        unreachable!("write_governance_updated: variant mismatch")
    };
    p.set_governance_key(governance_key);
    p.set_subkey_index(*subkey_index);
    p.set_lamport_ts(*lamport_ts);
}

pub(super) fn read_mek_rotated(
    p: cap::m_e_k_rotated_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
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
) -> Result<ControlPayload, ProtocolError> {
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
) -> Result<ControlPayload, ProtocolError> {
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
    }))
}

pub(super) fn read_mek_transfer_ack(
    p: cap::mek_transfer_ack_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
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
    }))
}

pub(super) fn read_admin_keypair_grant(
    p: cap::admin_keypair_grant_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::AdminKeypairGrant {
        wrapped_owner_keypair: p
            .get_wrapped_owner_keypair()
            .map_err(|e| capnp_err(&e))?
            .to_vec(),
        wrapped_slot_seed: p
            .get_wrapped_slot_seed()
            .map_err(|e| capnp_err(&e))?
            .to_vec(),
    })
}

pub(super) fn read_slot_keypair_grant(
    p: cap::slot_keypair_grant_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::SlotKeypairGrant {
        slot_index: p.get_slot_index(),
        segment_index: p.get_segment_index(),
        wrapped_slot_keypair: p
            .get_wrapped_slot_keypair()
            .map_err(|e| capnp_err(&e))?
            .to_vec(),
    })
}

pub(super) fn read_governance_updated(
    p: cap::governance_updated_payload::Reader<'_>,
) -> Result<ControlPayload, ProtocolError> {
    Ok(ControlPayload::GovernanceUpdated {
        governance_key: text_to_string(p.get_governance_key().map_err(|e| capnp_err(&e))?)?,
        subkey_index: p.get_subkey_index(),
        lamport_ts: p.get_lamport_ts(),
    })
}
