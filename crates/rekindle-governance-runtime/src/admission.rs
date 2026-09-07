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

use rekindle_governance::state::AdmissionDecision;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry};
use rekindle_types::id::PseudonymKey;

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// One pending request, flattened for a frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMember {
    pub pseudonym_hex: String,
    /// Peer-supplied. Untrusted display data — render escaped.
    pub display_name: String,
    pub lamport: u64,
}

/// Ask to be admitted.
///
/// Self-authored by construction: the entry names *us* as the requester,
/// and `validate_write` drops any `JoinRequested` whose author differs —
/// so this cannot be used to enqueue somebody else.
pub async fn request_join<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    display_name: &str,
) -> Result<(), GovernanceRuntimeError> {
    let me = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;

    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::JoinRequested {
            requester: PseudonymKey::from_hex_lossy(&me),
            display_name: display_name.to_string(),
            lamport,
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
    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::MemberApproved {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            lamport,
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
    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::MemberRejected {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            reason: reason.map(ToString::to_string),
            lamport,
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
            display_name: pending.display_name.clone(),
            lamport: pending.lamport,
        })
        .collect();
    // Deterministic order across peers: Lamport, then pseudonym to break
    // ties. A HashMap iteration order would differ per process and make
    // two clients disagree about "the next request".
    out.sort_by(|a, b| {
        a.lamport
            .cmp(&b.lamport)
            .then_with(|| a.pseudonym_hex.cmp(&b.pseudonym_hex))
    });
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
