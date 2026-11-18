//! Deps surface for the onboarding-answers handler: the AppState and
//! SQLite writes and the sync push live behind this sub-trait, implemented
//! by the Host's `HostGovernance`.
//!
//! `MembershipEventDeps: GovernanceRuntimeDeps` (supertrait) so the
//! onboarding handler can call `apply::write_entry` / `send_to_mesh` /
//! `governance_state` without re-declaring them here.

use crate::deps::GovernanceRuntimeDeps;

/// I/O capabilities the membership-event handlers need beyond what
/// `GovernanceRuntimeDeps` already provides.
pub trait MembershipEventDeps: GovernanceRuntimeDeps {
    // ---------- Onboarding ----------

    /// Mark the member's onboarding complete (in-memory when `is_self`,
    /// and the `community_members` row). Its roles are the merged role
    /// assignments, not stored here. Fire-and-forget SQLite write.
    fn persist_onboarding_completion(&self, community_id: &str, pseudonym_hex: &str, is_self: bool);

    /// Fire-and-forget: push our own onboarding completion into the
    /// personal SMPL ReadState so paired devices stop showing the wizard
    /// (architecture §28.4).
    fn push_onboarding_complete_to_sync(&self, community_id: &str);
}
