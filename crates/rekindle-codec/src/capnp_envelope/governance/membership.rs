//! Membership ops (plan E3.2): `MemberJoined`, `MemberRecordMoved`,
//! `MemberLeft` and the durable kick `MemberRemoved`.

use super::super::sub_types::{pseudonym_key_from_capnp, pseudonym_key_to_capnp};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::CodecError;
use rekindle_types::governance::{GovernanceEntry, InviteProof};
use rekindle_types::id::PseudonymKey;

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
    })
}

pub(super) fn read_member_removed(
    p: schema_pkg::member_removed_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::MemberRemoved {
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
) {
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
) {
    p.set_member_record(member_record);
    p.set_member_record_owner(member_record_owner);
}

pub(super) fn write_member_removed(
    mut p: schema_pkg::member_removed_payload::Builder<'_>,
    target: &PseudonymKey,
    reason: Option<&str>,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
}
