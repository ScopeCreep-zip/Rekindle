//! Timeout enforcement for the coordinator.
//!
//! Checks if a member is currently timed out and prevents them from
//! performing write actions.

use rekindle_protocol::dht::community::{
    permissions_v2::Permissions,
    types::{MemberSummary, RoleEntryV2},
};

/// Returns `true` if the member is currently timed out (and not ADMINISTRATOR).
pub fn is_timed_out(member: &MemberSummary, roles: &[RoleEntryV2], now_secs: u64) -> bool {
    // Administrators are exempt from timeouts
    for role_id in &member.role_ids {
        if let Some(role) = roles.iter().find(|r| r.id == *role_id) {
            let perms = Permissions::from_bits_truncate(role.permissions);
            if perms.contains(Permissions::ADMINISTRATOR) {
                return false;
            }
        }
    }

    // Check if timeout is active
    if let Some(timeout_until) = member.timeout_until {
        if now_secs < timeout_until {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_member(timeout_until: Option<u64>, role_ids: Vec<u32>) -> MemberSummary {
        MemberSummary {
            pseudonym_key: "alice".into(),
            display_name: "Alice".into(),
            role_ids,
            joined_at: 1000,
            subkey_index: 0,
            onboarding_complete: true,
            timeout_until,
        }
    }

    fn admin_role() -> RoleEntryV2 {
        RoleEntryV2 {
            id: 3,
            name: "Admin".into(),
            color: 0,
            permissions: Permissions::ADMINISTRATOR.bits(),
            position: 3,
            hoist: false,
            mentionable: false,
        }
    }

    fn member_role() -> RoleEntryV2 {
        RoleEntryV2 {
            id: 1,
            name: "Member".into(),
            color: 0,
            permissions: (Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES).bits(),
            position: 1,
            hoist: false,
            mentionable: false,
        }
    }

    #[test]
    fn not_timed_out() {
        let member = make_member(None, vec![1]);
        assert!(!is_timed_out(&member, &[member_role()], 5000));
    }

    #[test]
    fn actively_timed_out() {
        let member = make_member(Some(10000), vec![1]);
        assert!(is_timed_out(&member, &[member_role()], 5000));
    }

    #[test]
    fn timeout_expired() {
        let member = make_member(Some(3000), vec![1]);
        assert!(!is_timed_out(&member, &[member_role()], 5000));
    }

    #[test]
    fn admin_exempt_from_timeout() {
        let member = make_member(Some(10000), vec![3]);
        assert!(!is_timed_out(&member, &[admin_role()], 5000));
    }
}
