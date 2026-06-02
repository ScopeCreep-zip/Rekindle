//! `MemberRolesChanged` control-message handler (drained from
//! `legacy/membership/roles.rs`).

use crate::event::GovernanceRuntimeEvent;
use crate::membership_events::deps::MembershipEventDeps;

/// Apply a member's new role set: persist it (and `my_role_ids` if it's
/// us), then emit `MemberRolesChanged`.
pub fn process_member_roles_changed<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_hex: &str,
    role_ids: &[u32],
) {
    let is_self = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .as_deref()
        == Some(pseudonym_hex);

    deps.persist_member_roles(community_id, pseudonym_hex, role_ids, is_self);

    deps.emit_event(GovernanceRuntimeEvent::MemberRolesChanged {
        community_id: community_id.to_string(),
        pseudonym_hex: pseudonym_hex.to_string(),
        role_ids: role_ids.to_vec(),
    });
}
