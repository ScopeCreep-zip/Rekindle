//! Membership ops (plan E3.2): `MemberJoined`, `MemberRecordMoved`,
//! `MemberLeft` and the durable kick `MemberRemoved`.

use super::super::sub_types::pseudonym_key_from_capnp;
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::CodecError;
use rekindle_types::governance::{GovernanceEntry, InviteProof, SlotHead};

pub(super) fn read_member_joined(
    p: schema_pkg::member_joined_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    let invite_proof = if p.get_has_invite_proof() {
        Some(InviteProof {
            code_hash: text_to_string(p.get_invite_code_hash().map_err(|e| capnp_err(&e))?)?,
            sig: p.get_invite_sig().map_err(|e| capnp_err(&e))?.to_vec(),
        })
    } else {
        None
    };
    Ok(GovernanceEntry::MemberJoined {
        member_record: text_to_string(p.get_member_record().map_err(|e| capnp_err(&e))?)?,
        member_record_owner: text_to_string(
            p.get_member_record_owner().map_err(|e| capnp_err(&e))?,
        )?,
        invite_proof,
        owner_proof: p.get_owner_proof().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

pub(super) fn read_member_record_moved(
    p: schema_pkg::member_record_moved_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::MemberRecordMoved {
        member_record: text_to_string(p.get_member_record().map_err(|e| capnp_err(&e))?)?,
        member_record_owner: text_to_string(
            p.get_member_record_owner().map_err(|e| capnp_err(&e))?,
        )?,
        owner_proof: p.get_owner_proof().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

/// A `MemberLeft`: the leaver's own slot heads (A/B finding R3-8). The
/// list is required: an op that leaves it out states no heads at all, not
/// an empty set, and is refused (no build ever wrote that shape).
pub(super) fn read_member_left(
    p: schema_pkg::member_left_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    if !p.has_heads() {
        return Err(CodecError::Deserialization(
            "memberLeft without its slot heads".into(),
        ));
    }
    Ok(GovernanceEntry::MemberLeft {
        heads: read_heads(p.get_heads().map_err(|e| capnp_err(&e))?)?,
    })
}

fn read_heads(
    list: capnp::struct_list::Reader<'_, schema_pkg::slot_head_payload::Owned>,
) -> Result<Vec<SlotHead>, CodecError> {
    list.iter()
        .map(|h| {
            // A missing digest reads as empty and is refused with a short one.
            let digest = h.get_digest().map_err(|e| capnp_err(&e))?;
            let digest: [u8; 32] = digest.try_into().map_err(|_| {
                CodecError::Deserialization("slot head without a 32-byte digest".into())
            })?;
            Ok(SlotHead {
                record_key: text_to_string(h.get_record_key().map_err(|e| capnp_err(&e))?)?,
                seq: h.get_seq(),
                digest,
            })
        })
        .collect()
}

pub(super) fn read_member_removed(
    p: schema_pkg::member_removed_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::MemberRemoved {
        heads: read_heads(p.get_heads().map_err(|e| capnp_err(&e))?)?,
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

pub(super) fn write_member_joined(
    mut p: schema_pkg::member_joined_payload::Builder<'_>,
    member_record: &str,
    member_record_owner: &str,
    invite_proof: Option<&InviteProof>,
    owner_proof: &[u8],
) {
    p.set_owner_proof(owner_proof);
    p.set_member_record(member_record);
    p.set_member_record_owner(member_record_owner);
    p.set_has_invite_proof(invite_proof.is_some());
    if let Some(proof) = invite_proof {
        p.set_invite_code_hash(&proof.code_hash);
        p.set_invite_sig(&proof.sig);
    }
}

pub(super) fn write_member_record_moved(
    mut p: schema_pkg::member_record_moved_payload::Builder<'_>,
    member_record: &str,
    member_record_owner: &str,
    owner_proof: &[u8],
) {
    p.set_member_record(member_record);
    p.set_member_record_owner(member_record_owner);
    p.set_owner_proof(owner_proof);
}

pub(super) fn write_member_removed(
    mut p: schema_pkg::member_removed_payload::Builder<'_>,
    target: &rekindle_types::id::PseudonymKey,
    reason: Option<&str>,
    heads: &[SlotHead],
) {
    super::super::sub_types::pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    write_heads(
        p.init_heads(u32::try_from(heads.len()).unwrap_or(u32::MAX)),
        heads,
    );
}

pub(super) fn write_member_left(
    p: schema_pkg::member_left_payload::Builder<'_>,
    heads: &[SlotHead],
) {
    write_heads(
        p.init_heads(u32::try_from(heads.len()).unwrap_or(u32::MAX)),
        heads,
    );
}

fn write_heads(
    mut list: capnp::struct_list::Builder<'_, schema_pkg::slot_head_payload::Owned>,
    heads: &[SlotHead],
) {
    for (i, head) in (0u32..).zip(heads) {
        let mut h = list.reborrow().get(i);
        h.set_record_key(&head.record_key);
        h.set_seq(head.seq);
        h.set_digest(&head.digest);
    }
}
