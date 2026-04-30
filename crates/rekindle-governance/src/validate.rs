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

const MAX_STATIC_EMOJIS: usize = 50;
const MAX_ANIMATED_EMOJIS: usize = 50;
const MAX_STICKERS: usize = 30;
const MAX_SOUNDBOARD_SOUNDS: usize = 48;
const MAX_AUTOMOD_KEYWORDS: usize = 1000;
const MAX_AUTOMOD_REGEX_PATTERNS: usize = 10;

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

        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        }
        | GovernanceEntry::RoleUnassignment {
            target, role_id, ..
        } => has(perms, MANAGE_ROLES) || can_self_assign_role(writer, target, role_id, state),

        GovernanceEntry::BanEntry { .. } | GovernanceEntry::UnbanEntry { .. } => {
            has(perms, BAN_MEMBERS)
        }

        GovernanceEntry::TimeoutEntry { .. } | GovernanceEntry::RemoveTimeoutEntry { .. } => {
            has(perms, TIMEOUT_MEMBERS)
        }

        GovernanceEntry::CommunityMeta { .. } => has(perms, MANAGE_COMMUNITY),

        // MEK generation bumps use Max-Register (highest generation wins).
        // Rotator authority is verified by checking trigger_departed + cascade_skipped
        // against the deterministic rotator selection algorithm. However, since the
        // merge engine already enforces Max-Register (only highest generation survives),
        // a rogue bump to generation N is superseded by the legitimate bump to N+1.
        // Full rotator verification requires cross-referencing presence timestamps
        // (for cascade_skipped validation), which is done at the sync layer, not here.
        // At the governance CRDT layer, we enforce: writer is not banned (checked above).
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

        GovernanceEntry::ThreadArchived { .. } => has(perms, MANAGE_CHANNELS),

        GovernanceEntry::EventCreated { .. } => has(perms, CREATE_EVENTS),

        GovernanceEntry::EventArchived { .. } => has(perms, MANAGE_EVENTS),

        GovernanceEntry::ExpressionAdded { kind, animated, .. } => {
            (has(perms, MANAGE_EXPRESSIONS) || has(perms, CREATE_EXPRESSIONS))
                && expression_within_limits(kind, *animated, state)
        }

        GovernanceEntry::ExpressionRemoved { .. } => has(perms, MANAGE_EXPRESSIONS),

        GovernanceEntry::OnboardingConfig { .. } => has(perms, MANAGE_COMMUNITY),

        GovernanceEntry::WelcomeScreen { .. } => has(perms, MANAGE_COMMUNITY),

        GovernanceEntry::AdminDelete { .. } => has(perms, MANAGE_MESSAGES),

        // Segment expansion requires admin-level access
        GovernanceEntry::SegmentAdded { .. } => has(perms, MANAGE_COMMUNITY),

        GovernanceEntry::AutoModRule {
            rule_id,
            enabled,
            trigger_json,
            action,
            ..
        } => {
            has(perms, MANAGE_COMMUNITY)
                && validate_automod_rule(rule_id, *enabled, trigger_json, action, state)
        }

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

fn can_self_assign_role(
    writer: &PseudonymKey,
    target: &PseudonymKey,
    role_id: &rekindle_types::id::RoleId,
    state: &GovernanceState,
) -> bool {
    writer == target
        && state
            .roles
            .get(role_id)
            .map(|role| role.self_assignable)
            .unwrap_or(false)
}

fn expression_within_limits(kind: &str, animated: bool, state: &GovernanceState) -> bool {
    let static_emoji_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "emoji" && !expr.animated)
        .count();
    let animated_emoji_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "emoji" && expr.animated)
        .count();
    let sticker_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "sticker")
        .count();
    let soundboard_count = state
        .expressions
        .values()
        .filter(|expr| expr.kind == "soundboard")
        .count();

    match kind {
        "emoji" if animated => animated_emoji_count < MAX_ANIMATED_EMOJIS,
        "emoji" => static_emoji_count < MAX_STATIC_EMOJIS,
        "sticker" => sticker_count < MAX_STICKERS,
        "soundboard" => soundboard_count < MAX_SOUNDBOARD_SOUNDS,
        _ => false,
    }
}

fn validate_automod_rule(
    rule_id: &[u8; 16],
    enabled: bool,
    trigger_json: &str,
    action: &str,
    state: &GovernanceState,
) -> bool {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TriggerConfig {
        #[serde(default)]
        keywords: Vec<String>,
        #[serde(default)]
        regex_patterns: Vec<String>,
    }

    if !matches!(action, "block_locally" | "blur_content" | "alert_moderators") {
        return false;
    }

    let Ok(trigger) = serde_json::from_str::<TriggerConfig>(trigger_json) else {
        return false;
    };
    if trigger
        .regex_patterns
        .iter()
        .any(|pattern| regex::Regex::new(pattern).is_err())
    {
        return false;
    }

    let current_totals = state
        .automod_rules
        .iter()
        .filter(|(existing_id, rule)| *existing_id != rule_id && rule.enabled)
        .filter_map(|(_, rule)| serde_json::from_str::<TriggerConfig>(&rule.trigger_json).ok())
        .fold((0usize, 0usize), |(keywords, regexes), trigger| {
            (
                keywords + trigger.keywords.len(),
                regexes + trigger.regex_patterns.len(),
            )
        });

    let next_keywords = current_totals.0 + if enabled { trigger.keywords.len() } else { 0 };
    let next_regexes = current_totals.1 + if enabled { trigger.regex_patterns.len() } else { 0 };
    next_keywords <= MAX_AUTOMOD_KEYWORDS && next_regexes <= MAX_AUTOMOD_REGEX_PATTERNS
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
                self_assignable: false,
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
                self_assignable: false,
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
    fn mek_bump_accepted_at_governance_layer() {
        let state = state_with_creator_and_roles();
        let entry = GovernanceEntry::MEKGenerationBump {
            generation: 2,
            trigger_departed: pseudo(10),
            cascade_skipped: vec![],
            lamport: 10,
        };
        // At governance CRDT layer, MEK bumps are accepted (Max-Register).
        // Full rotator authority is verified at the sync layer.
        assert!(validate_write(&pseudo(99), &entry, &state));
    }

    #[test]
    fn banned_member_cannot_write() {
        let mut state = state_with_creator_and_roles();
        state.bans.insert(pseudo(50));

        let entry = GovernanceEntry::MEKGenerationBump {
            generation: 2,
            trigger_departed: pseudo(10),
            cascade_skipped: vec![],
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

    #[test]
    fn self_assignable_role_allows_self_write() {
        let mut state = state_with_creator_and_roles();
        let self_role_id = rid(9);
        state.roles.insert(
            self_role_id,
            RoleState {
                name: "self".into(),
                permissions: 0,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: true,
                lamport: 3,
            },
        );

        let entry = GovernanceEntry::RoleAssignment {
            target: pseudo(99),
            role_id: self_role_id,
            lamport: 10,
        };
        assert!(validate_write(&pseudo(99), &entry, &state));
    }

    #[test]
    fn expression_limit_rejects_extra_static_emoji() {
        let mut state = state_with_creator_and_roles();
        for index in 0..50_u8 {
            state.expressions.insert(
                [index; 16],
                crate::state::ExpressionState {
                    name: format!("emoji-{index}"),
                    kind: "emoji".into(),
                    content_hash: format!("hash-{index}"),
                    inline_data: Some(vec![index]),
                    animated: false,
                    tags: vec![],
                    lamport: u64::from(index),
                },
            );
        }

        let entry = GovernanceEntry::ExpressionAdded {
            expression_id: [99_u8; 16],
            name: "overflow".into(),
            kind: "emoji".into(),
            content_hash: "hash-overflow".into(),
            inline_data: Some(vec![1, 2, 3]),
            animated: false,
            tags: vec![],
            lamport: 50,
        };
        assert!(!validate_write(&pseudo(5), &entry, &state));
    }
}
