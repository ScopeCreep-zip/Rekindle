//! Reader-validates: check if a writer had permission for a governance entry.
//!
//! Every peer independently validates incoming governance entries against the
//! CRDT-merged permission state. Invalid entries are silently excluded from
//! the materialized view.
//!
//! See architecture doc §9.3 for the enforcement model.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::*;

use crate::permissions::compute_permissions;
use crate::state::GovernanceState;

/// Check if `writer` had permission to write `entry` given the current `state`.
///
/// Returns `true` if the entry should be included in the merged state.
/// Returns `false` if it should be silently excluded.
///
/// # Note on circular dependency
/// Role assignments determine who can make governance changes, but role
/// assignments ARE governance changes. The resolution: entries are processed
/// in Lamport order, and at each entry, the current accumulated permission
/// state is used. Genesis entries (first in order) bypass all checks.
pub fn validate_write(
    writer: &PseudonymKey,
    entry: &GovernanceEntry,
    state: &GovernanceState,
) -> bool {
    // Creator always passes validation
    if state.creator.as_ref() == Some(writer) {
        return true;
    }

    // Banned members can't write valid governance entries
    if state.bans.contains(writer) {
        return false;
    }

    let perms = compute_permissions(writer, None, state, 0);

    match entry {
        GovernanceEntry::ChannelCreated { .. }
        | GovernanceEntry::ChannelArchived { .. }
        | GovernanceEntry::ChannelUpdated { .. } => has(perms, MANAGE_CHANNELS),

        GovernanceEntry::RoleDefinition { .. } => has(perms, MANAGE_ROLES),

        GovernanceEntry::RoleAssignment { .. } | GovernanceEntry::RoleUnassignment { .. } => {
            has(perms, MANAGE_ROLES)
        }

        GovernanceEntry::BanEntry { .. } | GovernanceEntry::UnbanEntry { .. } => {
            has(perms, BAN_MEMBERS)
        }

        GovernanceEntry::TimeoutEntry { .. } => has(perms, TIMEOUT_MEMBERS),

        GovernanceEntry::CommunityMeta { .. } => has(perms, MANAGE_COMMUNITY),

        // Any member can bump MEK generation (deterministic rotator protocol)
        GovernanceEntry::MEKGenerationBump { .. } => true,

        GovernanceEntry::CategoryCreated { .. } | GovernanceEntry::CategoryArchived { .. } => {
            has(perms, MANAGE_CHANNELS)
        }

        GovernanceEntry::PermissionOverwrite { .. } => {
            has(perms, MANAGE_CHANNELS) || has(perms, MANAGE_ROLES)
        }

        GovernanceEntry::ThreadCreated { .. } => {
            has(perms, SEND_MESSAGES) // any member who can send can create threads
        }

        GovernanceEntry::EventCreated { .. } => has(perms, CREATE_EVENTS),

        GovernanceEntry::OnboardingConfig { .. } => has(perms, MANAGE_COMMUNITY),

        GovernanceEntry::AdminDelete { .. } => has(perms, MANAGE_MESSAGES),

        // Segment expansion requires admin-level access
        GovernanceEntry::SegmentAdded { .. } => has(perms, MANAGE_COMMUNITY),

        GovernanceEntry::AutoModRule { .. } => has(perms, MANAGE_COMMUNITY),

        GovernanceEntry::RoleArchived { .. } => has(perms, MANAGE_ROLES),

        GovernanceEntry::CategoryUpdated { .. } => has(perms, MANAGE_CHANNELS),

        GovernanceEntry::InviteCreated { .. } => has(perms, CREATE_INVITES),

        GovernanceEntry::InviteRevoked { .. } => has(perms, MANAGE_COMMUNITY),
    }
}

/// Check if a permission bitmask includes the required permission.
/// ADMINISTRATOR always passes.
fn has(perms: u64, required: u64) -> bool {
    (perms & ADMINISTRATOR != 0) || (perms & required == required)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{GovernanceState, RoleState};
    use std::collections::{HashMap, HashSet};

    fn pseudo(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    fn rid(b: u8) -> rekindle_types::id::RoleId {
        rekindle_types::id::RoleId([b; 16])
    }

    fn state_with_creator_and_roles() -> GovernanceState {
        let mut roles = HashMap::new();
        roles.insert(
            rid(0),
            RoleState {
                name: "everyone".into(),
                permissions: VIEW_CHANNELS | SEND_MESSAGES | READ_HISTORY,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                lamport: 1,
            },
        );
        roles.insert(
            rid(1),
            RoleState {
                name: "admin".into(),
                permissions: ADMINISTRATOR,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                lamport: 2,
            },
        );

        let mut assignments = HashMap::new();
        let mut admin_roles = HashSet::new();
        admin_roles.insert(rid(1));
        assignments.insert(pseudo(5), admin_roles); // pseudo(5) is admin

        GovernanceState {
            creator: Some(pseudo(1)),
            roles,
            role_assignments: assignments,
            ..Default::default()
        }
    }

    #[test]
    fn creator_always_validates() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: rekindle_types::id::ChannelId([0; 16]),
            name: "test".into(),
            channel_type: "text".into(),
            record_key: "k".into(),
            category_id: None,
            position: 0,
            lamport: 10,
        };
        assert!(validate_write(&pseudo(1), &entry, &state));
    }

    #[test]
    fn regular_member_cannot_manage_channels() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: rekindle_types::id::ChannelId([0; 16]),
            name: "test".into(),
            channel_type: "text".into(),
            record_key: "k".into(),
            category_id: None,
            position: 0,
            lamport: 10,
        };
        // pseudo(99) has only @everyone perms — no MANAGE_CHANNELS
        assert!(!validate_write(&pseudo(99), &entry, &state));
    }

    #[test]
    fn admin_can_manage_channels() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: rekindle_types::id::ChannelId([0; 16]),
            name: "test".into(),
            channel_type: "text".into(),
            record_key: "k".into(),
            category_id: None,
            position: 0,
            lamport: 10,
        };
        // pseudo(5) is admin
        assert!(validate_write(&pseudo(5), &entry, &state));
    }

    #[test]
    fn anyone_can_bump_mek() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::MEKGenerationBump {
            generation: 2,
            lamport: 10,
        };
        // Even a regular member can bump MEK (deterministic rotator protocol)
        assert!(validate_write(&pseudo(99), &entry, &state));
    }

    #[test]
    fn banned_member_cannot_write() {
        let mut state = state_with_creator_and_roles();
        state.bans.insert(pseudo(50));

        let entry = GovernanceEntry::MEKGenerationBump {
            generation: 2,
            lamport: 10,
        };
        assert!(!validate_write(&pseudo(50), &entry, &state));
    }

    #[test]
    fn regular_member_can_create_thread() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::ThreadCreated {
            thread_id: rekindle_types::id::ThreadId([0; 16]),
            parent_channel_id: rekindle_types::id::ChannelId([0; 16]),
            name: "discussion".into(),
            record_key: None,
            lamport: 10,
        };
        // @everyone has SEND_MESSAGES
        assert!(validate_write(&pseudo(99), &entry, &state));
    }
}
