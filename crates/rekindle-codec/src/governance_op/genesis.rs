//! The wire form of the community's `GenesisCert` (plan E3.2, ADR 0011;
//! `schemas/governance_op.capnp` `GenesisCert`): the anchor's subkey 0.
//!
//! Both signatures cover `GOV_GENESIS ‖ anchor ‖ unsigned`, where
//! `unsigned` is [`genesis_unsigned_bytes`]: the cert and the anchor owner
//! re-encoded canonically with both signatures unset. [`decode_genesis`]
//! re-encodes that form from the decoded value, so a reader verifies the
//! bytes the signers signed however the writer laid out its message.

use rekindle_types::dht_layout::member_record::MAX_SUBKEY_VALUE_LEN;
use rekindle_types::governance::{AdmissionMode, GenesisCert};
use rekindle_types::id::{PseudonymKey, RoleId};

use super::{data_field, fixed};
use crate::capnp_codec::{capnp_err, pack, text_to_string, unpack_bounded};
use crate::community_governance_capnp::AdmissionMode as WireMode;
use crate::governance_op_capnp::genesis_cert;
use crate::member_record::{check_len, check_size};
use crate::CodecError;

/// Largest encoded cert: one subkey value.
pub const GENESIS_CERT_MAX_BYTES: usize = MAX_SUBKEY_VALUE_LEN;

/// Most initial roles a cert names.
pub const GENESIS_MAX_ROLES: usize = 64;

const MAX_WORDS: usize = GENESIS_CERT_MAX_BYTES / 8 * 4;

/// A cert as the anchor holds it: the cert, the anchor owner `K_A`'s public
/// key and both signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedGenesis {
    pub cert: GenesisCert,
    pub anchor_owner: [u8; 32],
    pub anchor_sig: [u8; 64],
    pub creator_sig: [u8; 64],
}

/// A decoded cert and the canonical unsigned bytes its signatures cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedGenesis {
    pub signed: SignedGenesis,
    pub unsigned: Vec<u8>,
}

/// Encode a signed cert, refusing one over the bounds.
pub fn encode_genesis(signed: &SignedGenesis) -> Result<Vec<u8>, CodecError> {
    check_len(
        "genesis initial roles",
        signed.cert.initial_roles.len(),
        GENESIS_MAX_ROLES,
    )?;
    let out = encode(&signed.cert, &signed.anchor_owner, Some(signed));
    check_len("genesis cert", out.len(), GENESIS_CERT_MAX_BYTES)?;
    Ok(out)
}

/// The canonical unsigned bytes of `cert` under anchor owner `anchor_owner`.
#[must_use]
pub fn genesis_unsigned_bytes(cert: &GenesisCert, anchor_owner: &[u8; 32]) -> Vec<u8> {
    encode(cert, anchor_owner, None)
}

/// Decode a cert read from the anchor, with its canonical unsigned bytes.
pub fn decode_genesis(data: &[u8]) -> Result<DecodedGenesis, CodecError> {
    check_size("genesis cert", data, GENESIS_CERT_MAX_BYTES)?;
    let message = unpack_bounded(data, MAX_WORDS)?;
    let r = message
        .get_root::<genesis_cert::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let roles_r = r.get_initial_roles().map_err(|e| capnp_err(&e))?;
    let role_count = usize::try_from(roles_r.len()).unwrap_or(usize::MAX);
    check_len("genesis initial roles", role_count, GENESIS_MAX_ROLES)?;
    let initial_roles = roles_r
        .iter()
        .map(|role| Ok(RoleId(fixed(data_field(role)?, "genesis role id")?)))
        .collect::<Result<Vec<_>, CodecError>>()?;
    let policy = match r
        .get_policy()
        .map_err(|e| CodecError::Deserialization(format!("genesis policy: unknown arm {}", e.0)))?
    {
        WireMode::Open => AdmissionMode::Open,
        WireMode::ApprovalRequired => AdmissionMode::ApprovalRequired,
    };
    let cert = GenesisCert {
        name: text_to_string(r.get_name().map_err(|e| capnp_err(&e))?)?,
        policy,
        initial_roles,
        creator_pseudonym: PseudonymKey(fixed(
            data_field(r.get_creator_pseudonym())?,
            "genesis creator pseudonym",
        )?),
        creator_member_record: text_to_string(
            r.get_creator_member_record().map_err(|e| capnp_err(&e))?,
        )?,
        creator_member_record_owner: text_to_string(
            r.get_creator_member_record_owner()
                .map_err(|e| capnp_err(&e))?,
        )?,
        created_at: r.get_created_at(),
    };
    let anchor_owner = fixed(data_field(r.get_anchor_owner())?, "genesis anchor owner")?;
    let unsigned = genesis_unsigned_bytes(&cert, &anchor_owner);
    let signed = SignedGenesis {
        anchor_sig: fixed(data_field(r.get_anchor_sig())?, "genesis anchor sig")?,
        creator_sig: fixed(data_field(r.get_creator_sig())?, "genesis creator sig")?,
        cert,
        anchor_owner,
    };
    Ok(DecodedGenesis { signed, unsigned })
}

/// One encoder for both forms, so they differ only in the signatures.
fn encode(cert: &GenesisCert, anchor_owner: &[u8; 32], sigs: Option<&SignedGenesis>) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    let mut b = builder.init_root::<genesis_cert::Builder<'_>>();
    b.set_name(cert.name.as_str());
    b.set_policy(match cert.policy {
        AdmissionMode::Open => WireMode::Open,
        AdmissionMode::ApprovalRequired => WireMode::ApprovalRequired,
    });
    let count = u32::try_from(cert.initial_roles.len()).unwrap_or(u32::MAX);
    let mut roles = b.reborrow().init_initial_roles(count);
    for (i, role) in (0..count).zip(&cert.initial_roles) {
        roles.set(i, &role.0);
    }
    b.set_creator_pseudonym(&cert.creator_pseudonym.0);
    b.set_creator_member_record(cert.creator_member_record.as_str());
    b.set_creator_member_record_owner(cert.creator_member_record_owner.as_str());
    b.set_created_at(cert.created_at);
    b.set_anchor_owner(anchor_owner);
    if let Some(signed) = sigs {
        b.set_anchor_sig(&signed.anchor_sig);
        b.set_creator_sig(&signed.creator_sig);
    }
    pack(&builder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed() -> SignedGenesis {
        SignedGenesis {
            cert: GenesisCert {
                name: "c".into(),
                policy: AdmissionMode::ApprovalRequired,
                initial_roles: vec![RoleId([0; 16]), RoleId([0xFF; 16])],
                creator_pseudonym: PseudonymKey([1; 32]),
                creator_member_record: "VLD0:m".into(),
                creator_member_record_owner: "VLD0:o".into(),
                created_at: 7,
            },
            anchor_owner: [2; 32],
            anchor_sig: [3; 64],
            creator_sig: [4; 64],
        }
    }

    #[test]
    fn round_trips_with_the_unsigned_form() {
        let s = signed();
        let decoded = decode_genesis(&encode_genesis(&s).unwrap()).unwrap();
        assert_eq!(decoded.signed, s);
        assert_eq!(
            decoded.unsigned,
            genesis_unsigned_bytes(&s.cert, &s.anchor_owner)
        );
    }

    #[test]
    fn a_short_signature_is_refused() {
        let mut builder = capnp::message::Builder::new_default();
        let mut b = builder.init_root::<genesis_cert::Builder<'_>>();
        b.set_creator_pseudonym(&[1; 32]);
        b.set_anchor_owner(&[2; 32]);
        b.set_anchor_sig(&[3; 10]);
        b.set_creator_sig(&[4; 64]);
        assert!(decode_genesis(&pack(&builder)).is_err());
    }

    #[test]
    fn too_many_roles_are_refused() {
        let mut s = signed();
        s.cert.initial_roles = vec![RoleId([0; 16]); GENESIS_MAX_ROLES + 1];
        assert!(encode_genesis(&s).is_err());
    }
}
