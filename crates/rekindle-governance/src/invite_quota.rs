//! M10.3 — Invite quota enforcement (architecture §20.6 line 2607).
//!
//! Reader-validates: every honest peer applies the same per-inviter
//! active-invite cap and the same per-invite max-uses cap. A rogue admin
//! who mints over-quota invites sees their entries silently excluded
//! from the merged governance state on every peer.
//!
//! This is a Tier 6 helper — pure logic, no I/O, no async, no Veilid.

use rekindle_types::id::PseudonymKey;

use crate::state::GovernanceState;

/// Hard ceiling on per-invite reuse; it lives in `rekindle-types` so the
/// CLI refuses an out-of-range `--max-uses` with the same bound.
pub use rekindle_types::invite::MAX_USES_PER_INVITE;

/// Hard ceiling on simultaneously-active invites per inviter. Deliberately
/// decoupled from `CommunityPolicy.max_joins_per_interval` (which bounds the
/// raid-detection JOIN rate, not a record count): every live invite is a
/// governance `InviteCreated` pointer entry occupying the author's SMPL subkey,
/// so the count must be bounded independently of the join-rate policy to keep
/// the subkey under Veilid's ~4112 B per-subkey cap. `compact_author_entries`
/// enforces the same bound on the write path (keep-newest-N).
pub const MAX_ACTIVE_INVITES_PER_INVITER: u32 = 8;

/// Count active (non-revoked, non-expired) invites whose `creator_pseudonym`
/// matches `inviter`. Excludes the entry currently being validated by
/// `code_hash` — `validate_write` calls this BEFORE the entry has been
/// merged, so the count reflects the prior state.
///
/// Active = present in `state.invites` (revoked invites are removed at
/// merge time). The expiry check is left to the joiner-side
/// `find_invite_in_governance` flow because expiry is wall-clock dependent.
pub fn active_invites_by_inviter(state: &GovernanceState, inviter: &PseudonymKey) -> u32 {
    state
        .invites
        .values()
        .filter(|invite| invite.creator_pseudonym == *inviter)
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

/// Check whether `inviter` may mint a new invite. Returns `true` if accept,
/// `false` if reject.
///
/// The cap is the fixed `MAX_ACTIVE_INVITES_PER_INVITER` — independent of
/// `CommunityPolicy.max_joins_per_interval` (which governs the raid-detection
/// join rate, a different concern). The count stays clock-free (incl. expired
/// invites) so `validate_write` converges across peers; expired invites are
/// shed on the author's next write by `compact_author_entries`.
pub fn check_active_invites_cap(state: &GovernanceState, inviter: &PseudonymKey) -> bool {
    active_invites_by_inviter(state, inviter) < MAX_ACTIVE_INVITES_PER_INVITER
}

/// Reject invites whose declared reuse count exceeds the hard cap.
/// Reader-validates: even a creator-bypass writer cannot smuggle a
/// `max_uses = u32::MAX` entry past honest peers. `max_uses = 0` is not
/// "unlimited": unlimited invites are forbidden, so 0 is refused too.
pub fn check_max_uses_cap(max_uses: u32) -> bool {
    (1..=MAX_USES_PER_INVITE).contains(&max_uses)
}

#[cfg(test)]
#[path = "invite_quota_tests.rs"]
mod tests;
