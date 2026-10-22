//! Moderation: ban, unban, timeout, lifting a timeout. A removal writes
//! no entry (the Host's `remove_member`: the `Kick` notice, the member
//! forgotten, the departure rotation).
//!
//! Pure orchestration over `GovernanceRuntimeDeps`, so both tracks drive
//! one implementation. The Tauri host already wrote these as governance
//! entries (`community_moderation_runtime.rs`); the daemon still routed
//! them through coordinator RPCs that mutated the member index directly.
//! Two implementations of one rule set, and only one of them was v2.0.
//!
//! Every function here writes an entry and stops. Enforcement is
//! reader-side: each peer merges the entry, checks the author's
//! permissions, and drops it if they lack them. A banned member can keep
//! writing to their own subkey forever — SMPL gives them that by
//! construction — and honest peers simply ignore everything they wrote
//! after the ban's lamport.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{BAN_MEMBERS, TIMEOUT_MEMBERS};

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// Ban a member. Requires `BAN_MEMBERS`.
///
/// The local permission check is a courtesy that fails fast with a clear
/// error; it is not the enforcement. Readers validate independently, so
/// a client that skipped this check gains nothing.
pub async fn ban_member<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
    reason: Option<&str>,
) -> Result<(), GovernanceRuntimeError> {
    deps.require_permission(community_id, BAN_MEMBERS)?;
    let lamport = deps.next_governance_lamport(community_id)?;
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::BanEntry {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            reason: reason.map(ToString::to_string),
            lamport,
        },
    )
    .await?;

    // A departure invalidates the current text MEK for everyone else:
    // the banned member still holds it. Fire-and-forget — the ban itself
    // has already landed and must not be undone by a rotation failure.
    deps.spawn_text_mek_rotation_for_ban(community_id, pseudonym_hex);
    Ok(())
}

/// Lift a ban. Requires `BAN_MEMBERS`.
///
/// The merge requires a strictly higher lamport than the ban it
/// reverses, which `next_governance_lamport` guarantees for our own writes.
pub async fn unban_member<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
) -> Result<(), GovernanceRuntimeError> {
    deps.require_permission(community_id, BAN_MEMBERS)?;
    let lamport = deps.next_governance_lamport(community_id)?;
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::UnbanEntry {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            lamport,
        },
    )
    .await
}

/// Temporarily strip a member's permissions. Requires `TIMEOUT_MEMBERS`,
/// the bit readers check the entry against (`validate_write`).
pub async fn timeout_member<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
    duration_seconds: u64,
    reason: Option<&str>,
) -> Result<(), GovernanceRuntimeError> {
    deps.require_permission(community_id, TIMEOUT_MEMBERS)?;
    let lamport = deps.next_governance_lamport(community_id)?;
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::TimeoutEntry {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            duration_seconds,
            reason: reason.map(ToString::to_string),
            // Wall-clock start, so every peer computes the same expiry
            // from the entry alone rather than from when it merged it.
            started_at: deps.clock().now_secs(),
            lamport,
        },
    )
    .await
}

/// Lift a timeout early. Requires `TIMEOUT_MEMBERS`.
pub async fn remove_timeout<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
) -> Result<(), GovernanceRuntimeError> {
    deps.require_permission(community_id, TIMEOUT_MEMBERS)?;
    let lamport = deps.next_governance_lamport(community_id)?;
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RemoveTimeoutEntry {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            lamport,
        },
    )
    .await
}
