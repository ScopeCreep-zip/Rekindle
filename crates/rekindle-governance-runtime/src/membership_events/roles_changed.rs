//! `MemberRolesChanged` control-message handler (drained from
//! `legacy/membership/roles.rs`).

use crate::event::GovernanceRuntimeEvent;
use crate::membership_events::deps::MembershipEventDeps;

/// Announce a member's new role set as `MemberRolesChanged`. Nothing is
/// stored: every reader derives roles from the merged role assignments.
pub fn process_member_roles_changed<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
    role_ids: &[u32],
) {
    deps.emit_event(GovernanceRuntimeEvent::MemberRolesChanged {
        community_id: community_id.to_string(),
        pseudonym_hex: pseudonym_hex.to_string(),
        role_ids: role_ids.to_vec(),
    });
}
