//! [`VerifiedOp`]: a governance op whose signature, chain position and
//! dependencies are checked, with its hash and its validated Lamport value
//! (plan E3.2). `merge` takes only these.

use rekindle_secrets::sign::verify_signature;
use rekindle_types::domains;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

/// The order every CRDT apply rule compares: the op's validated Lamport
/// value, then its hash. Total, and the same on every reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OpStamp {
    pub lamport: u64,
    pub hash: OpHash,
}

impl OpStamp {
    /// Below every real op (whose lamport is at least 1).
    pub const MIN: Self = Self {
        lamport: 0,
        hash: OpHash([0; 32]),
    };
}

/// Why an op was refused before it could reach `merge`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpRejection {
    BadSignature,
    BadInviteProof,
    /// A dep (or the author's previous op) is not known yet: the op waits
    /// in `pending_deps` until it is fetched by its `OpRef`.
    MissingDep(OpHash),
    /// A dep's locator disagrees with the op it names.
    DepMismatch(OpHash),
    /// `prev` is absent at seq > 0, present at seq 0, not the author's
    /// `seq - 1`, or not among the deps.
    BadChain,
}

/// An op whose signature and deps are checked. Built only by
/// [`VerifiedOp::verify`].
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedOp {
    op: GovernanceOp,
    hash: OpHash,
    lamport: u64,
}

/// The op hash: BLAKE3 `derive_key(GOV_OP_HASH)` over the canonical
/// unsigned bytes and the signature.
#[must_use]
pub fn op_hash(unsigned: &[u8], sig: &[u8; 64]) -> OpHash {
    let mut hasher = blake3::Hasher::new_derive_key(domains::GOV_OP_HASH);
    hasher.update(unsigned);
    hasher.update(sig);
    OpHash(*hasher.finalize().as_bytes())
}

/// The bytes an op's author signs: `GOV_OP ‖ anchor ‖ unsigned`.
#[must_use]
pub fn op_signing_bytes(anchor: &str, unsigned: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(domains::GOV_OP.len() + anchor.len() + unsigned.len());
    out.extend_from_slice(domains::GOV_OP.as_bytes());
    out.extend_from_slice(anchor.as_bytes());
    out.extend_from_slice(unsigned);
    out
}

/// The bytes a joiner signs for its invite proof:
/// `INVITE_PROOF ‖ anchor ‖ code_hash ‖ member_record`.
#[must_use]
pub fn invite_proof_bytes(anchor: &str, code_hash: &str, member_record: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(domains::INVITE_PROOF.as_bytes());
    out.extend_from_slice(anchor.as_bytes());
    out.extend_from_slice(code_hash.as_bytes());
    out.extend_from_slice(member_record.as_bytes());
    out
}

impl VerifiedOp {
    /// Verify `op` of community `anchor`. `unsigned` is the canonical
    /// unsigned encoding the op was decoded from (the codec produces both
    /// from one buffer); `known` resolves an already verified op by hash.
    pub fn verify<'a>(
        anchor: &str,
        op: GovernanceOp,
        unsigned: &[u8],
        known: impl Fn(&OpHash) -> Option<&'a Self>,
    ) -> Result<Self, OpRejection> {
        verify_signature(&op.author.0, &op_signing_bytes(anchor, unsigned), &op.sig)
            .map_err(|_| OpRejection::BadSignature)?;
        check_invite_proof(anchor, &op)?;
        let mut lamport = 0;
        for dep in &op.deps {
            let found = known(&dep.op_hash).ok_or(OpRejection::MissingDep(dep.op_hash))?;
            if found.op.author != dep.author || found.op.seq != dep.author_seq {
                return Err(OpRejection::DepMismatch(dep.op_hash));
            }
            lamport = lamport.max(found.lamport);
        }
        check_chain(&op, &known)?;
        let hash = op_hash(unsigned, &op.sig);
        Ok(Self {
            op,
            hash,
            lamport: lamport + 1,
        })
    }

    #[must_use]
    pub fn op(&self) -> &GovernanceOp {
        &self.op
    }

    #[must_use]
    pub fn author(&self) -> &PseudonymKey {
        &self.op.author
    }

    #[must_use]
    pub fn body(&self) -> &GovernanceEntry {
        &self.op.body
    }

    #[must_use]
    pub fn hash(&self) -> OpHash {
        self.hash
    }

    /// `1 + max(dep lamports)`, 1 with no deps.
    #[must_use]
    pub fn lamport(&self) -> u64 {
        self.lamport
    }

    #[must_use]
    pub fn stamp(&self) -> OpStamp {
        OpStamp {
            lamport: self.lamport,
            hash: self.hash,
        }
    }

    /// The locator other ops cite this one by.
    #[must_use]
    pub fn op_ref(&self, author_record: &str) -> OpRef {
        OpRef {
            op_hash: self.hash,
            author: self.op.author.clone(),
            author_record: author_record.to_owned(),
            author_seq: self.op.seq,
        }
    }

    /// A fixture op for this crate's tests, whose signature is not
    /// checked; `lamport` and `hash` are taken as given.
    #[cfg(test)]
    pub(crate) fn assume_verified(op: GovernanceOp, hash: OpHash, lamport: u64) -> Self {
        Self { op, hash, lamport }
    }
}

/// The author's chain: seq 0 has no `prev`; seq n > 0 cites its own
/// seq n - 1 as `prev`, and that op is among the deps.
fn check_chain<'a>(
    op: &GovernanceOp,
    known: &impl Fn(&OpHash) -> Option<&'a VerifiedOp>,
) -> Result<(), OpRejection> {
    match (op.seq, op.prev) {
        (0, None) => Ok(()),
        (seq, Some(prev)) if seq > 0 => {
            let found = known(&prev).ok_or(OpRejection::MissingDep(prev))?;
            let chained = found.op.author == op.author && found.op.seq + 1 == seq;
            let cited = op.deps.iter().any(|d| d.op_hash == prev);
            if chained && cited {
                Ok(())
            } else {
                Err(OpRejection::BadChain)
            }
        }
        _ => Err(OpRejection::BadChain),
    }
}

/// A `MemberJoined` invite proof must be signed by the joiner over the
/// invite commitment and its own member record. Whether the commitment
/// names a live invite under its cap is a merge-time, causal check.
fn check_invite_proof(anchor: &str, op: &GovernanceOp) -> Result<(), OpRejection> {
    let GovernanceEntry::MemberJoined {
        member_record,
        invite_proof: Some(proof),
        ..
    } = &op.body
    else {
        return Ok(());
    };
    let sig: [u8; 64] = proof
        .sig
        .as_slice()
        .try_into()
        .map_err(|_| OpRejection::BadInviteProof)?;
    let bytes = invite_proof_bytes(anchor, &proof.code_hash, member_record);
    verify_signature(&op.author.0, &bytes, &sig).map_err(|_| OpRejection::BadInviteProof)
}
