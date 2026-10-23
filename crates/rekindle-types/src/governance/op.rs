//! Governance ops (plan E3.2, ADR 0011): each [`GovernanceEntry`] travels
//! inside a [`GovernanceOp`] that its author pseudonym signs and appends to
//! its own member record. Ops form a DAG through `deps`; the op's Lamport
//! value is derived from it (`1 + max(dep lamports)`), never carried.
//!
//! Wire form: `schemas/governance_op.capnp`.

use serde::{Deserialize, Serialize};

use crate::id::{PseudonymKey, RoleId};

use super::{AdmissionMode, GovernanceEntry};

/// BLAKE3 hash of an op's signed bytes (`domains::GOV_OP_HASH`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OpHash(pub [u8; 32]);

/// A locator for an op: enough to fetch it from its author's member
/// record (`authorRecord`, at `authorSeq`) and to check what came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpRef {
    pub op_hash: OpHash,
    pub author: PseudonymKey,
    /// DHT key of the author's member record.
    pub author_record: String,
    pub author_seq: u64,
}

/// One signed governance op.
#[derive(Debug, Clone, PartialEq)]
pub struct GovernanceOp {
    pub author: PseudonymKey,
    /// Position in the author's own op chain, from 0.
    pub seq: u64,
    /// The author's previous op (`seq - 1`); `None` only at seq 0.
    pub prev: Option<OpHash>,
    /// Causal dependencies: the heads the author had seen.
    pub deps: Vec<OpRef>,
    /// The author's wall-clock seconds at issue (a tie-break only, never
    /// trusted for authority).
    pub issued_at: u64,
    pub body: GovernanceEntry,
    /// Ed25519 by `author` over `GOV_OP ‖ anchor ‖ canonical unsigned bytes`.
    pub sig: [u8; 64],
}

/// A joiner's proof that it holds an invite: the invite code's commitment
/// (`InviteCreated.code_hash`) and the joiner pseudonym's signature over
/// `INVITE_PROOF ‖ anchor ‖ code_hash ‖ member_record`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteProof {
    pub code_hash: String,
    /// 64-byte Ed25519 signature; any other length fails verification.
    pub sig: Vec<u8>,
}

/// The community's genesis certificate, written once to the anchor record
/// and pinned by every member (S5 creates and verifies it at origin). The
/// creator is its `creator_pseudonym` — MSC4289's immutable creator — and no
/// op at any lamport can take that place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisCert {
    pub name: String,
    pub policy: AdmissionMode,
    pub initial_roles: Vec<RoleId>,
    pub creator_pseudonym: PseudonymKey,
    pub creator_member_record: String,
    pub creator_member_record_owner: String,
    pub created_at: u64,
}
