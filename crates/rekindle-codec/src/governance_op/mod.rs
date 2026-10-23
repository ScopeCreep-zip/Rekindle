//! The wire form of a signed governance op (ADR 0011, plan E3.2;
//! `schemas/governance_op.capnp`).
//!
//! The author signs `GOV_OP ‖ anchor ‖ unsigned`, where `unsigned` is the
//! op re-encoded canonically without its signature
//! ([`op_unsigned_bytes`]). [`decode_op`] returns that canonical form
//! beside the op, re-encoded from the decoded value rather than sliced from
//! the input, so a reader hands `rekindle_governance::VerifiedOp::verify`
//! exactly the bytes the author signed however the sender laid out its
//! message. An unknown or retired body arm is
//! [`CodecError::UnknownGovernanceArm`], which the caller quarantines.

#[cfg(test)]
mod tests;

use rekindle_types::dht_layout::member_record::{MAX_FRONTIER_REFS, MAX_SUBKEY_VALUE_LEN};
use rekindle_types::governance::{GovernanceOp, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use crate::capnp_codec::{capnp_err, pack, text_to_string, unpack_bounded};
use crate::capnp_envelope::{read_governance_entry, write_governance_entry};
use crate::governance_op_capnp::{governance_op, op_ref};
use crate::member_record::{check_len, check_size};
use crate::CodecError;

/// Largest encoded op: one member-record subkey, where an op is published.
pub const GOVERNANCE_OP_MAX_BYTES: usize = MAX_SUBKEY_VALUE_LEN;

/// Most deps an op cites: the heads its author had seen, which is at most
/// one `Frontier` value.
pub const GOVERNANCE_OP_MAX_DEPS: usize = MAX_FRONTIER_REFS;

/// Traversal bound for the unpacked message: packing collapses zero words,
/// so the unpacked form may exceed the packed bytes; four times the cap in
/// words leaves room for any op that fits the cap.
const MAX_WORDS: usize = GOVERNANCE_OP_MAX_BYTES / 8 * 4;

/// A decoded op and the canonical unsigned bytes its signature covers.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedOp {
    pub op: GovernanceOp,
    pub unsigned: Vec<u8>,
}

/// Encode a signed op, refusing one over [`GOVERNANCE_OP_MAX_BYTES`] or
/// with more than [`GOVERNANCE_OP_MAX_DEPS`] deps.
pub fn encode_op(op: &GovernanceOp) -> Result<Vec<u8>, CodecError> {
    check_len("governance op deps", op.deps.len(), GOVERNANCE_OP_MAX_DEPS)?;
    let out = encode(op, true);
    check_len("governance op", out.len(), GOVERNANCE_OP_MAX_BYTES)?;
    Ok(out)
}

/// The canonical unsigned bytes of `op`: the bytes its author signs (after
/// `GOV_OP ‖ anchor`) and the op hash covers (before the signature).
#[must_use]
pub fn op_unsigned_bytes(op: &GovernanceOp) -> Vec<u8> {
    encode(op, false)
}

/// Decode an op off the network, with its canonical unsigned bytes.
pub fn decode_op(data: &[u8]) -> Result<DecodedOp, CodecError> {
    check_size("governance op", data, GOVERNANCE_OP_MAX_BYTES)?;
    let message = unpack_bounded(data, MAX_WORDS)?;
    let r = message
        .get_root::<governance_op::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let deps_r = r.get_deps().map_err(|e| capnp_err(&e))?;
    let dep_count = usize::try_from(deps_r.len()).unwrap_or(usize::MAX);
    if dep_count > GOVERNANCE_OP_MAX_DEPS {
        return Err(CodecError::Deserialization(format!(
            "governance op has {dep_count} deps, the bound is {GOVERNANCE_OP_MAX_DEPS}"
        )));
    }
    let deps = deps_r.iter().map(read_op_ref).collect::<Result<_, _>>()?;
    let prev = data_field(r.get_prev())?;
    let prev = if prev.is_empty() {
        None
    } else {
        Some(OpHash(fixed(prev, "governance op prev")?))
    };
    let body = read_governance_entry(r.get_body().map_err(|e| capnp_err(&e))?)?;
    let op = GovernanceOp {
        author: PseudonymKey(fixed(data_field(r.get_author())?, "governance op author")?),
        seq: r.get_seq(),
        prev,
        deps,
        issued_at: r.get_issued_at(),
        body,
        sig: fixed(data_field(r.get_sig())?, "governance op sig")?,
    };
    let unsigned = op_unsigned_bytes(&op);
    Ok(DecodedOp { op, unsigned })
}

/// One encoder for both forms, so they differ only in `sig`.
fn encode(op: &GovernanceOp, signed: bool) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    let mut b = builder.init_root::<governance_op::Builder<'_>>();
    b.set_author(&op.author.0);
    b.set_seq(op.seq);
    if let Some(prev) = op.prev {
        b.set_prev(&prev.0);
    }
    let count = u32::try_from(op.deps.len()).unwrap_or(u32::MAX);
    let mut deps = b.reborrow().init_deps(count);
    for (i, dep) in (0..count).zip(&op.deps) {
        write_op_ref(deps.reborrow().get(i), dep);
    }
    b.set_issued_at(op.issued_at);
    write_governance_entry(b.reborrow().init_body(), &op.body);
    if signed {
        b.set_sig(&op.sig);
    }
    pack(&builder)
}

fn write_op_ref(mut b: op_ref::Builder<'_>, r: &OpRef) {
    b.set_op_hash(&r.op_hash.0);
    b.set_author(&r.author.0);
    b.set_author_record(r.author_record.as_str());
    b.set_author_seq(r.author_seq);
}

fn read_op_ref(r: op_ref::Reader<'_>) -> Result<OpRef, CodecError> {
    Ok(OpRef {
        op_hash: OpHash(fixed(data_field(r.get_op_hash())?, "op ref hash")?),
        author: PseudonymKey(fixed(data_field(r.get_author())?, "op ref author")?),
        author_record: text_to_string(r.get_author_record().map_err(|e| capnp_err(&e))?)?,
        author_seq: r.get_author_seq(),
    })
}

fn data_field(get: Result<&[u8], capnp::Error>) -> Result<&[u8], CodecError> {
    get.map_err(|e| capnp_err(&e))
}

/// A fixed-length byte field; any other length is refused.
fn fixed<const N: usize>(bytes: &[u8], what: &str) -> Result<[u8; N], CodecError> {
    bytes.try_into().map_err(|_| {
        CodecError::Deserialization(format!("{what} is {} bytes, not {N}", bytes.len()))
    })
}
