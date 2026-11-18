//! The wire form of a signed governance op (ADR 0011, plan E3.2;
//! `schemas/governance_op.capnp`).
//!
//! The author signs `GOV_OP ‖ anchor ‖ unsigned`, where `unsigned` is the
//! op in Cap'n Proto canonical form without its signature
//! ([`op_unsigned_bytes`]). [`read_op`] computes that form from the
//! received message itself (copied, `sig` cleared, canonicalized), so a
//! reader hands `rekindle_governance::VerifiedOp::verify` exactly the bytes
//! the author signed however the sender laid out its message, and can check
//! the signature of an op it cannot interpret: one of an arm beyond its
//! schema, or carrying fields it does not know (a newer version's op,
//! [`ReadOp::Unknown`]), which the caller quarantines with that reason (D15).
//! A retired body arm is [`CodecError::UnknownGovernanceArm`].

pub mod genesis;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unknown_tests;

use rekindle_types::dht_layout::member_record::{MAX_FRONTIER_REFS, MAX_SUBKEY_VALUE_LEN};
use rekindle_types::governance::{Frontier, GovernanceOp, OpHash, OpRef, RosterChecksum};
use rekindle_types::id::PseudonymKey;

use crate::capnp_codec::{bounds, capnp_err, pack, text_to_string, unpack_bounded};
use crate::capnp_envelope::{read_governance_entry, write_governance_entry};
use crate::governance_op_capnp::{frontier, governance_op, op_ref};
use crate::member_record::{check_len, check_size};
use crate::CodecError;

/// Largest encoded op: one member-record subkey, where an op is published.
pub const GOVERNANCE_OP_MAX_BYTES: usize = MAX_SUBKEY_VALUE_LEN;

/// Most deps an op cites: the heads its author had seen, which is at most
/// one `Frontier` value.
pub const GOVERNANCE_OP_MAX_DEPS: usize = MAX_FRONTIER_REFS;

/// A decoded op and the canonical unsigned bytes its signature covers.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedOp {
    pub op: GovernanceOp,
    pub unsigned: Vec<u8>,
}

/// What a reader could not interpret in a signed op (D15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Uninterpretable {
    /// A body arm beyond the reader's schema, by its number.
    Kind(u16),
    /// Fields the reader's schema does not have: its canonical unsigned
    /// form re-encoded from the decoded value is not the received one.
    Version,
}

/// A signed op the reader cannot interpret: its envelope, the canonical
/// unsigned bytes its signature covers, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownOp {
    pub author: PseudonymKey,
    pub seq: u64,
    pub issued_at: u64,
    pub sig: [u8; 64],
    pub unsigned: Vec<u8>,
    pub what: Uninterpretable,
}

/// A decoded op: interpreted, or not interpretable by this reader.
#[derive(Debug, Clone, PartialEq)]
pub enum ReadOp {
    Known(Box<DecodedOp>),
    Unknown(UnknownOp),
}

/// Encode a signed op, refusing one over [`GOVERNANCE_OP_MAX_BYTES`] or
/// with more than [`GOVERNANCE_OP_MAX_DEPS`] deps.
pub fn encode_op(op: &GovernanceOp) -> Result<Vec<u8>, CodecError> {
    check_len("governance op deps", op.deps.len(), GOVERNANCE_OP_MAX_DEPS)?;
    seal_op(&op_unsigned_bytes(op), &op.sig)
}

/// Seal an op's canonical unsigned bytes (as [`op_unsigned_bytes`] or
/// [`read_op`] give them) and its signature into the wire form, refusing
/// one over [`GOVERNANCE_OP_MAX_BYTES`]. The bytes are read back through
/// the op's bounded reader and copied as they are, fields beyond this
/// writer's schema included, so the sealed op reads back with exactly
/// those unsigned bytes and `sig`: the one sealing path [`encode_op`] uses.
pub fn seal_op(unsigned: &[u8], sig: &[u8; 64]) -> Result<Vec<u8>, CodecError> {
    let message = unpack_bounded(&packed_literal(unsigned)?, bounds::GOVERNANCE_OP_BOUND)?;
    let r = message
        .get_root::<governance_op::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let mut builder = capnp::message::Builder::new_default();
    builder.set_root(r).map_err(|e| capnp_err(&e))?;
    let mut root = builder
        .get_root::<governance_op::Builder<'_>>()
        .map_err(|e| capnp_err(&e))?;
    root.set_sig(sig);
    let out = pack(&builder);
    check_len("governance op", out.len(), GOVERNANCE_OP_MAX_BYTES)?;
    Ok(out)
}

/// A single-segment message (the canonical form's one segment) in packed
/// framing, every word written literally: a `0xff` tag, the word, then a
/// count of up to 255 further words copied raw (Cap'n Proto encoding spec,
/// "Packing"). A valid packed stream that `unpack_bounded` reads.
fn packed_literal(segment: &[u8]) -> Result<Vec<u8>, CodecError> {
    if !segment.len().is_multiple_of(8) {
        return Err(CodecError::Deserialization(format!(
            "governance op segment is {} bytes, not whole words",
            segment.len()
        )));
    }
    let words = u32::try_from(segment.len() / 8).map_err(|_| {
        CodecError::Deserialization("governance op segment is too large".to_owned())
    })?;
    let mut header = 0u32.to_le_bytes().to_vec();
    header.extend_from_slice(&words.to_le_bytes());
    let mut out = Vec::with_capacity(segment.len() + segment.len() / 1024 + 14);
    // The reader takes the segment table apart from the segment, so no run
    // crosses from one into the other.
    for run in std::iter::once(header.as_slice()).chain(segment.chunks(256 * 8)) {
        let (first, rest) = run.split_at(8);
        out.push(0xff);
        out.extend_from_slice(first);
        out.push(u8::try_from(rest.len() / 8).unwrap_or(u8::MAX));
        out.extend_from_slice(rest);
    }
    Ok(out)
}

/// The canonical unsigned bytes of `op`: the bytes its author signs (after
/// `GOV_OP ‖ anchor`) and the op hash covers (before the signature).
#[must_use]
pub fn op_unsigned_bytes(op: &GovernanceOp) -> Vec<u8> {
    let builder = build(op);
    let root = builder.get_root_as_reader::<governance_op::Reader<'_>>();
    // A builder's own root always reads back and canonicalizes.
    root.and_then(canonical).unwrap_or_default()
}

/// Decode an op off the network, with its canonical unsigned bytes; an op
/// this reader cannot interpret is [`CodecError::UnknownGovernanceArm`]
/// (see [`read_op`] for the op itself).
pub fn decode_op(data: &[u8]) -> Result<DecodedOp, CodecError> {
    match read_op(data)? {
        ReadOp::Known(decoded) => Ok(*decoded),
        ReadOp::Unknown(UnknownOp { what, .. }) => {
            Err(CodecError::UnknownGovernanceArm(match what {
                Uninterpretable::Kind(arm) => format!("governance entry arm {arm}"),
                Uninterpretable::Version => "fields beyond this reader's schema".to_owned(),
            }))
        }
    }
}

/// Decode an op off the network: [`ReadOp::Known`] with its canonical
/// unsigned bytes, or [`ReadOp::Unknown`] when its body arm is beyond this
/// reader's schema or it carries fields the schema does not have (D15).
pub fn read_op(data: &[u8]) -> Result<ReadOp, CodecError> {
    check_size("governance op", data, GOVERNANCE_OP_MAX_BYTES)?;
    let message = unpack_bounded(data, bounds::GOVERNANCE_OP_BOUND)?;
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
    let author = PseudonymKey(fixed(data_field(r.get_author())?, "governance op author")?);
    let sig = fixed(data_field(r.get_sig())?, "governance op sig")?;
    let unsigned = received_unsigned(r).map_err(|e| capnp_err(&e))?;
    let unknown = |what| {
        Ok(ReadOp::Unknown(UnknownOp {
            author: author.clone(),
            seq: r.get_seq(),
            issued_at: r.get_issued_at(),
            sig,
            unsigned: unsigned.clone(),
            what,
        }))
    };
    let body_r = r.get_body().map_err(|e| capnp_err(&e))?;
    if let Err(capnp::NotInSchema(arm)) = body_r.which() {
        return unknown(Uninterpretable::Kind(arm));
    }
    let body = match read_governance_entry(body_r) {
        Ok(body) => body,
        // An enum value or nested arm beyond the schema inside a known arm.
        Err(CodecError::UnknownVariant(_)) => return unknown(Uninterpretable::Version),
        Err(e) => return Err(e),
    };
    let op = GovernanceOp {
        author: author.clone(),
        seq: r.get_seq(),
        prev,
        deps,
        issued_at: r.get_issued_at(),
        body,
        sig,
    };
    if op_unsigned_bytes(&op) != unsigned {
        return unknown(Uninterpretable::Version);
    }
    Ok(ReadOp::Known(Box::new(DecodedOp { op, unsigned })))
}

/// The canonical unsigned form of a received op: the message copied as it
/// came (fields beyond this reader's schema included), `sig` cleared, then
/// canonicalized, so it equals the author's [`op_unsigned_bytes`].
fn received_unsigned(r: governance_op::Reader<'_>) -> capnp::Result<Vec<u8>> {
    let mut copy = capnp::message::Builder::new_default();
    copy.set_root(r)?;
    let root = copy.get_root::<governance_op::Builder<'_>>()?;
    let mut fields: capnp::dynamic_struct::Builder<'_> =
        capnp::dynamic_value::Builder::from(root).downcast();
    fields.clear_named("sig")?;
    canonical(copy.get_root_as_reader::<governance_op::Reader<'_>>()?)
}

/// `r` in Cap'n Proto canonical form (one segment, preorder, truncated),
/// as bytes.
fn canonical(r: governance_op::Reader<'_>) -> capnp::Result<Vec<u8>> {
    // One first segment that holds it all (plus the root pointer): the
    // canonical form is a single segment.
    let words = u32::try_from(r.total_size()?.word_count + 1).unwrap_or(u32::MAX);
    let mut out = capnp::message::Builder::new(
        capnp::message::HeapAllocator::new().first_segment_words(words),
    );
    out.set_root_canonical(r)?;
    Ok(out
        .get_segments_for_output()
        .first()
        .map(|s| s.to_vec())
        .unwrap_or_default())
}

/// Encode a member record's `Frontier` (`M`/3), refusing more than
/// [`MAX_FRONTIER_REFS`] heads.
pub fn encode_frontier(value: &Frontier) -> Result<Vec<u8>, CodecError> {
    check_len("frontier heads", value.heads.len(), MAX_FRONTIER_REFS)?;
    let mut builder = capnp::message::Builder::new_default();
    let mut b = builder.init_root::<frontier::Builder<'_>>();
    b.set_roster_checksum(&value.roster_checksum.0);
    let count = u32::try_from(value.heads.len()).unwrap_or(u32::MAX);
    let mut list = b.init_heads(count);
    for (i, head) in (0..count).zip(&value.heads) {
        write_op_ref(list.reborrow().get(i), head);
    }
    let out = pack(&builder);
    check_len("frontier", out.len(), MAX_SUBKEY_VALUE_LEN)?;
    Ok(out)
}

/// Decode a member record's `Frontier` off the network; a checksum that is
/// not 32 bytes is refused.
pub fn decode_frontier(data: &[u8]) -> Result<Frontier, CodecError> {
    check_size("frontier", data, MAX_SUBKEY_VALUE_LEN)?;
    let message = unpack_bounded(data, bounds::GOVERNANCE_OP_BOUND)?;
    let r = message
        .get_root::<frontier::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let heads = r.get_heads().map_err(|e| capnp_err(&e))?;
    let count = usize::try_from(heads.len()).unwrap_or(usize::MAX);
    if count > MAX_FRONTIER_REFS {
        return Err(CodecError::Deserialization(format!(
            "frontier has {count} heads, the bound is {MAX_FRONTIER_REFS}"
        )));
    }
    let roster_checksum = RosterChecksum(fixed(
        data_field(r.get_roster_checksum())?,
        "frontier roster checksum",
    )?);
    Ok(Frontier {
        heads: heads.iter().map(read_op_ref).collect::<Result<_, _>>()?,
        roster_checksum,
    })
}

/// The op without `sig`; [`seal_op`] adds it to the canonical form.
fn build(op: &GovernanceOp) -> capnp::message::Builder<capnp::message::HeapAllocator> {
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
    builder
}

pub(crate) fn write_op_ref(mut b: op_ref::Builder<'_>, r: &OpRef) {
    b.set_op_hash(&r.op_hash.0);
    b.set_author(&r.author.0);
    b.set_author_record(r.author_record.as_str());
    b.set_author_seq(r.author_seq);
}

pub(crate) fn read_op_ref(r: op_ref::Reader<'_>) -> Result<OpRef, CodecError> {
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
