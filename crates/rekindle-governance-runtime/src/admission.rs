//! Admission: join requests and the decisions that settle them.
//!
//! Pure orchestration over `GovernanceRuntimeDeps`, so both tracks drive
//! the same code — the Tauri host and the daemon, and therefore any
//! frontend on the IPC bus.
//!
//! **These write decisions; they do not grant access.** Under SMPL every
//! slot-seed holder can write to their own subkey regardless, so
//! admission is enforced the way every other rule here is: each reader
//! validates and honest peers do not count an unapproved member. Jami
//! works the same way — a join commit missing its `/invited`
//! precondition is rejected by every peer, not blocked at the source.
//!
//! Under [`AdmissionMode::Open`] nothing calls into here: a joiner
//! claims a slot without asking, so the pending list stays empty and the
//! approve/reject paths are inert.

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::AdmissionDecision;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry, InviteProof};
use rekindle_types::id::PseudonymKey;

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// One pending member, flattened for a frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMember {
    pub pseudonym_hex: String,
    /// The name on the joiner's verified card, when we hold it.
    pub display_name: Option<String>,
    /// The `MemberJoined` op's stamp: the order every reader agrees on.
    pub stamp: OpStamp,
}

/// Announce our membership: our `MemberJoined` op, naming our member
/// record and its owner. Self-authored by construction (the op's author is
/// the joiner); under `ApprovalRequired` it stays pending until a
/// `MemberApproved`.
pub async fn announce_joined<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    member_record: &str,
    member_record_owner: &str,
    invite_proof: Option<InviteProof>,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::MemberJoined {
            member_record: member_record.to_string(),
            member_record_owner: member_record_owner.to_string(),
            invite_proof,
        },
    )
    .await
}

/// Admit a pending member. Requires `KICK_MEMBERS`, enforced by every
/// reader rather than here — a local check would only be a courtesy.
pub async fn approve_member<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::MemberApproved {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
        },
    )
    .await
}

/// Refuse a pending member.
pub async fn reject_member<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
    reason: Option<&str>,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::MemberRejected {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            reason: reason.map(ToString::to_string),
        },
    )
    .await
}

/// Everyone currently awaiting a decision, oldest first.
///
/// Read out of merged CRDT state, not a registry subkey: the queue is
/// governance, and every peer computes the same list from the same
/// entries.
#[must_use]
pub fn pending_members<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Vec<PendingMember> {
    let Some(state) = deps.governance_state(community_id) else {
        return Vec::new();
    };
    let mut out: Vec<PendingMember> = state
        .pending_members
        .iter()
        .map(|(pseudonym, pending)| PendingMember {
            pseudonym_hex: hex::encode(pseudonym.0),
            display_name: deps.member_card_name(community_id, pseudonym),
            stamp: pending.stamp,
        })
        .collect();
    // Deterministic order across peers: the op stamp (lamport, hash). A
    // HashMap iteration order would differ per process.
    out.sort_by_key(|m| m.stamp);
    out
}

/// Whether this community gates admission at all.
#[must_use]
pub fn admission_mode<D: GovernanceRuntimeDeps>(deps: &D, community_id: &str) -> AdmissionMode {
    deps.governance_state(community_id)
        .map(|s| s.effective_admission_mode())
        .unwrap_or_default()
}

/// The recorded decision for one pseudonym, if any.
#[must_use]
pub fn decision_for<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
) -> Option<AdmissionDecision> {
    let state = deps.governance_state(community_id)?;
    state
        .admitted
        .get(&PseudonymKey::from_hex_lossy(pseudonym_hex))
        .copied()
}
