//! Permission computation from CRDT-merged governance state.
//!
//! Implements the 8-step Discord-compatible resolution algorithm
//! from architecture doc §9.2. All permission checks in the v2.0
//! system go through `compute_permissions()`.

use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::*;

use crate::state::GovernanceState;

/// Compute effective permissions for a member in a specific context.
///
/// # Arguments
/// * `member` — The member's pseudonym key.
/// * `channel_id` — If Some, apply per-channel overwrites. If None, compute base perms.
/// * `state` — The CRDT-merged governance state.
/// * `now_unix` — Current unix timestamp for timeout expiry checks.
///
/// # Returns
/// A u64 bitmask of effective permissions.
pub fn compute_permissions(
    member: &PseudonymKey,
    channel_id: Option<&ChannelId>,
    state: &GovernanceState,
    now_unix: u64,
) -> u64 {
    // Step 0: Creator always has all permissions
    if state.creator.as_ref() == Some(member) {
        return ALL;
    }

    // Step 1: Start with @everyone role permissions
    let everyone_role_id = RoleId([0u8; 16]);
    let mut perms = state
        .roles
        .get(&everyone_role_id)
        .map(|r| r.permissions)
        .unwrap_or(0);

    // Step 2: OR all of member's assigned role permissions
    if let Some(member_roles) = state.role_assignments.get(member) {
        for role_id in member_roles {
            if *role_id == everyone_role_id {
                continue; // already included
            }
            if let Some(role) = state.roles.get(role_id) {
                perms |= role.permissions;
            }
        }
    }

    // Step 3: ADMINISTRATOR bypass — return all permissions
    if perms & ADMINISTRATOR != 0 {
        return ALL;
    }

    // Steps 4-6: Channel-specific overwrites (only if channel specified)
    if let Some(ch_id) = channel_id {
        // Step 4: @everyone channel overwrites
        let everyone_key = (*ch_id, format!("{everyone_role_id:?}"));
        if let Some(ow) = state.overwrites.get(&everyone_key) {
            perms &= !ow.deny;
            perms |= ow.allow;
        }

        // Step 5: Role channel overwrites (union allows, then apply denies)
        let mut role_allow: u64 = 0;
        let mut role_deny: u64 = 0;
        if let Some(member_roles) = state.role_assignments.get(member) {
            for role_id in member_roles {
                let key = (*ch_id, format!("{role_id:?}"));
                if let Some(ow) = state.overwrites.get(&key) {
                    role_allow |= ow.allow;
                    role_deny |= ow.deny;
                }
            }
        }
        perms = (perms & !role_deny) | role_allow;

        // Step 6: Member-specific channel overwrites (highest priority)
        let member_hex = hex_encode_pseudonym(member);
        let member_key = (*ch_id, member_hex);
        if let Some(ow) = state.overwrites.get(&member_key) {
            perms &= !ow.deny;
            perms |= ow.allow;
        }
    }

    // Step 7: Timed-out members get only VIEW_CHANNELS | READ_HISTORY
    if let Some(timeout) = state.timeouts.get(member) {
        if !timeout.is_expired(now_unix) {
            perms = VIEW_CHANNELS | READ_HISTORY;
        }
    }

    // Step 8: Implicit permission dependencies
    if perms & SEND_MESSAGES == 0 {
        perms &= !MENTION_EVERYONE;
        perms &= !ATTACH_FILES;
        perms &= !EMBED_LINKS;
    }
    if perms & VIEW_CHANNELS == 0 {
        perms = 0;
    }
    if perms & CONNECT == 0 {
        perms &= !SPEAK;
        perms &= !MUTE_MEMBERS;
        perms &= !DEAFEN_MEMBERS;
        perms &= !USE_VOICE_ACTIVITY;
        perms &= !PRIORITY_SPEAKER;
        perms &= !STREAM;
    }

    perms
}

fn hex_encode_pseudonym(key: &PseudonymKey) -> String {
    key.0.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{GovernanceState, RoleState, TimeoutState};
    use std::collections::{HashMap, HashSet};

    fn pseudo(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    fn rid(b: u8) -> RoleId {
        RoleId([b; 16])
    }

    fn base_state() -> GovernanceState {
        let everyone_role = RoleState {
            name: "everyone".into(),
            permissions: DEFAULT_EVERYONE,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            lamport: 1,
        };
        let mut roles = HashMap::new();
        roles.insert(rid(0), everyone_role);

        GovernanceState {
            creator: Some(pseudo(1)),
            roles,
            ..Default::default()
        }
    }

    #[test]
    fn creator_has_all() {
        let state = base_state();
        let perms = compute_permissions(&pseudo(1), None, &state, 0);
        assert_eq!(perms, ALL);
    }

    #[test]
    fn everyone_gets_defaults() {
        let state = base_state();
        let perms = compute_permissions(&pseudo(99), None, &state, 0);
        assert!(perms & SEND_MESSAGES != 0);
        assert!(perms & VIEW_CHANNELS != 0);
        assert!(perms & READ_HISTORY != 0);
        assert_eq!(perms & BAN_MEMBERS, 0);
        assert_eq!(perms & ADMINISTRATOR, 0);
    }

    #[test]
    fn administrator_bypasses_all() {
        let mut state = base_state();
        let admin_role = RoleState {
            name: "admin".into(),
            permissions: ADMINISTRATOR,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            lamport: 2,
        };
        state.roles.insert(rid(1), admin_role);
        let mut assignments = HashSet::new();
        assignments.insert(rid(1));
        state.role_assignments.insert(pseudo(5), assignments);

        let perms = compute_permissions(&pseudo(5), None, &state, 0);
        assert_eq!(perms, ALL);
    }

    #[test]
    fn timeout_strips_permissions() {
        let mut state = base_state();
        state.timeouts.insert(
            pseudo(10),
            TimeoutState {
                duration_seconds: 3600,
                started_at: 1000,
                lamport: 5,
            },
        );

        // Not expired (now=1500, expires at 4600)
        let perms = compute_permissions(&pseudo(10), None, &state, 1500);
        assert!(perms & VIEW_CHANNELS != 0);
        assert!(perms & READ_HISTORY != 0);
        assert_eq!(perms & SEND_MESSAGES, 0);
        assert_eq!(perms & BAN_MEMBERS, 0);

        // Expired (now=5000)
        let perms = compute_permissions(&pseudo(10), None, &state, 5000);
        assert!(perms & SEND_MESSAGES != 0, "timeout expired, perms restored");
    }

    #[test]
    fn no_view_channels_strips_everything() {
        let mut state = base_state();
        // Override @everyone to have no VIEW_CHANNELS
        state.roles.insert(
            rid(0),
            RoleState {
                name: "everyone".into(),
                permissions: SEND_MESSAGES, // has send but NOT view
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                lamport: 1,
            },
        );

        let perms = compute_permissions(&pseudo(99), None, &state, 0);
        assert_eq!(perms, 0, "no VIEW_CHANNELS → everything stripped");
    }

    #[test]
    fn implicit_deps_no_send_strips_mention() {
        let mut state = base_state();
        state.roles.insert(
            rid(0),
            RoleState {
                name: "everyone".into(),
                permissions: VIEW_CHANNELS | READ_HISTORY | MENTION_EVERYONE, // mention but no send
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                lamport: 1,
            },
        );

        let perms = compute_permissions(&pseudo(99), None, &state, 0);
        assert_eq!(perms & MENTION_EVERYONE, 0, "no SEND → MENTION stripped");
    }
}
