//! Moderation: ban, unban, timeout, lifting a timeout, the durable kick
//! (`MemberRemoved`) and our own leave (`MemberLeft`), plan E3.2 and
//! `evidence/slot-integrity.md` §4.4 ("Leave = `MemberLeft` op … Kick and
//! ban are moderator ops"). The merge drops a removed or departed member
//! from `members`, so no reader counts them as a key reader.
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
use rekindle_types::permissions::{BAN_MEMBERS, KICK_MEMBERS, TIMEOUT_MEMBERS};

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
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::BanEntry {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            reason: reason.map(ToString::to_string),
        },
    )
    .await?;

    // A departure invalidates the current text MEK for everyone else:
    // the banned member still holds it. Fire-and-forget — the ban itself
    // has already landed and must not be undone by a rotation failure.
    deps.spawn_text_mek_rotation_for_ban(community_id, pseudonym_hex);
    Ok(())
}

/// Remove a member without barring return: the durable kick
/// `MemberRemoved`, which readers accept from a `KICK_MEMBERS` holder who
/// outranks the target, and the departure rotation (ADR 0018) that the
/// removal drives.
pub async fn remove_member<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
) -> Result<(), GovernanceRuntimeError> {
    deps.require_permission(community_id, KICK_MEMBERS)?;
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::MemberRemoved {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
            reason: None,
        },
    )
    .await?;
    deps.spawn_text_mek_rotation_for_ban(community_id, pseudonym_hex);
    Ok(())
}

/// Our own departure: the `MemberLeft` op on our member record, written
/// before the community is forgotten here.
pub async fn leave<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(deps, community_id, GovernanceEntry::MemberLeft {}).await
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
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::UnbanEntry {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
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
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::RemoveTimeoutEntry {
            target: PseudonymKey::from_hex_lossy(pseudonym_hex),
        },
    )
    .await
}
