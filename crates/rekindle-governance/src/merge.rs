//! CRDT merge engine — the core of flat governance.
//!
//! Takes all `GovernanceEntry` variants from all member subkeys,
//! processes them in deterministic order, and produces a `GovernanceState`.
//!
//! **Convergence guarantee:** Given the same set of entries (in any order),
//! `merge()` always produces an identical `GovernanceState`. This is verified
//! by property-based tests.
//!
//! **Genesis bypass:** The first entry (lowest lamport) is always accepted
//! regardless of permissions — this bootstraps the community before any
//! role structure exists.
//!
//! **Reader-validates:** After genesis, each entry is checked against the
//! accumulated permission state. Entries from members without the required
//! permission are silently excluded.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::state::*;
use crate::validate::validate_write;

/// A single entry tagged with its author's pseudonym.
#[derive(Debug, Clone)]
pub struct AuthoredEntry {
    pub author: PseudonymKey,
    pub entry: GovernanceEntry,
}

/// Merge governance entries from all member subkeys into a canonical state.
///
/// # Arguments
/// * `subkeys` — One vec of entries per member subkey. Each tuple is
///   `(author_pseudonym, entries_from_that_subkey)`.
///
/// # Returns
/// A deterministic `GovernanceState` that all peers agree on.
pub fn merge(subkeys: &[(PseudonymKey, Vec<GovernanceEntry>)]) -> GovernanceState {
    // 1. Collect all entries with their author
    let mut all: Vec<AuthoredEntry> = Vec::new();
    for (author, entries) in subkeys {
        for entry in entries {
            all.push(AuthoredEntry {
                author: author.clone(),
                entry: entry.clone(),
            });
        }
    }

    // 2. Sort by (lamport, author_pseudonym) for deterministic total order
    all.sort_by(|a, b| {
        a.entry
            .lamport()
            .cmp(&b.entry.lamport())
            .then_with(|| a.author.0.cmp(&b.author.0))
    });

    // 3. Process in order, applying CRDT rules
    let mut state = GovernanceState::default();

    for (idx, authored) in all.iter().enumerate() {
        let is_genesis = idx == 0;

        if is_genesis {
            // Genesis entry always accepted — bootstraps the community
            state.creator = Some(authored.author.clone());
            apply(&authored.author, &authored.entry, &mut state);
        } else if validate_write(&authored.author, &authored.entry, &state) {
            apply(&authored.author, &authored.entry, &mut state);
        }
        // else: silently excluded (reader-validates)
    }

    state
}

/// Apply a single governance entry to the accumulated state.
///
/// Each entry type has its own CRDT merge rule (see architecture doc §4.4).
/// Public as `apply_entry` for incremental local updates (after permission validation).
pub fn apply_entry(author: &PseudonymKey, entry: &GovernanceEntry, state: &mut GovernanceState) {
    apply(author, entry, state);
}

/// Apply a single governance entry to the accumulated state.
///
/// Each entry type has its own CRDT merge rule (see architecture doc §4.4).
fn apply(_author: &PseudonymKey, entry: &GovernanceEntry, state: &mut GovernanceState) {
    match entry {
        // ── Channels: OR-Set (created minus archived) ──
        GovernanceEntry::ChannelCreated {
            channel_id,
            name,
            channel_type,
            record_key,
            category_id,
            position,
            lamport,
        } => {
            state.channels.insert(
                *channel_id,
                ChannelState {
                    name: name.clone(),
                    channel_type: channel_type.clone(),
                    record_key: record_key.clone(),
                    category_id: *category_id,
                    position: *position,
                    topic: None,
                    slowmode_seconds: None,
                    nsfw: None,
                    created_lamport: *lamport,
                },
            );
        }

        GovernanceEntry::ChannelArchived {
            channel_id,
            lamport,
        } => {
            // Only archive if lamport > creation lamport
            if let Some(ch) = state.channels.get(channel_id) {
                if *lamport > ch.created_lamport {
                    state.channels.remove(channel_id);
                }
            }
        }

        GovernanceEntry::ChannelUpdated {
            channel_id,
            name,
            topic,
            position,
            slowmode_seconds,
            nsfw,
            category_id,
            ..
        } => {
            if let Some(ch) = state.channels.get_mut(channel_id) {
                if let Some(n) = name {
                    ch.name.clone_from(n);
                }
                if topic.is_some() {
                    ch.topic.clone_from(topic);
                }
                if let Some(p) = position {
                    ch.position = *p;
                }
                if slowmode_seconds.is_some() {
                    ch.slowmode_seconds = *slowmode_seconds;
                }
                if nsfw.is_some() {
                    ch.nsfw = *nsfw;
                }
                if let Some(cat) = category_id {
                    ch.category_id = *cat;
                }
            }
        }

        // ── Roles: LWW per role_id ──
        GovernanceEntry::RoleDefinition {
            role_id,
            name,
            permissions,
            position,
            color,
            hoist,
            mentionable,
            self_assignable,
            lamport,
        } => {
            let existing_lamport = state.roles.get(role_id).map(|r| r.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.roles.insert(
                    *role_id,
                    RoleState {
                        name: name.clone(),
                        permissions: *permissions,
                        position: *position,
                        color: *color,
                        hoist: *hoist,
                        mentionable: *mentionable,
                        self_assignable: *self_assignable,
                        lamport: *lamport,
                    },
                );
            }
        }

        // ── Role assignments: LWW-Flag per (target, role_id) ──
        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        } => {
            state
                .role_assignments
                .entry(target.clone())
                .or_default()
                .insert(*role_id);
        }

        GovernanceEntry::RoleUnassignment {
            target, role_id, ..
        } => {
            if let Some(roles) = state.role_assignments.get_mut(target) {
                roles.remove(role_id);
            }
        }

        // ── Bans: LWW-Flag per target pseudonym ──
        GovernanceEntry::BanEntry {
            target, lamport, ..
        } => {
            let prev = state.ban_lamports.get(target).copied().unwrap_or(0);
            if *lamport > prev {
                state.bans.insert(target.clone());
                state.ban_lamports.insert(target.clone(), *lamport);
            }
        }

        GovernanceEntry::UnbanEntry { target, lamport } => {
            let prev = state.ban_lamports.get(target).copied().unwrap_or(0);
            if *lamport > prev {
                state.bans.remove(target);
                state.ban_lamports.insert(target.clone(), *lamport);
            }
        }

        // ── Timeout ──
        GovernanceEntry::TimeoutEntry {
            target,
            duration_seconds,
            started_at,
            lamport,
            ..
        } => {
            let existing_lamport = state.timeouts.get(target).map(|t| t.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.timeouts.insert(
                    target.clone(),
                    TimeoutState {
                        duration_seconds: *duration_seconds,
                        started_at: *started_at,
                        lamport: *lamport,
                    },
                );
            }
        }

        GovernanceEntry::RemoveTimeoutEntry { target, lamport } => {
            let existing_lamport = state.timeouts.get(target).map(|t| t.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.timeouts.remove(target);
            }
        }

        // ── Metadata: LWW ──
        GovernanceEntry::CommunityMeta {
            name,
            description,
            icon_hash,
            banner_hash,
            lamport,
        } => {
            let existing_lamport = state.metadata.as_ref().map(|m| m.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.metadata = Some(MetadataState {
                    name: name.clone().unwrap_or_default(),
                    description: description.clone(),
                    icon_hash: icon_hash.clone(),
                    banner_hash: banner_hash.clone(),
                    lamport: *lamport,
                });
            }
        }

        // ── MEK: Max-Register ──
        GovernanceEntry::MEKGenerationBump { generation, .. } => {
            if *generation > state.mek_generation {
                state.mek_generation = *generation;
            }
        }

        // ── Categories: OR-Set ──
        GovernanceEntry::CategoryCreated {
            category_id,
            name,
            position,
            lamport,
        } => {
            state.categories.insert(
                *category_id,
                CategoryState {
                    name: name.clone(),
                    position: *position,
                    created_lamport: *lamport,
                },
            );
        }

        GovernanceEntry::CategoryArchived {
            category_id,
            lamport,
        } => {
            if let Some(cat) = state.categories.get(category_id) {
                if *lamport > cat.created_lamport {
                    state.categories.remove(category_id);
                }
            }
        }

        // ── Permission overwrites: LWW per (channel, target) ──
        GovernanceEntry::PermissionOverwrite {
            channel_id,
            target_type,
            target_id,
            allow,
            deny,
            lamport,
        } => {
            let key = (*channel_id, target_id.clone());
            let existing_lamport = state.overwrites.get(&key).map(|o| o.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.overwrites.insert(
                    key,
                    OverwriteState {
                        target_type: target_type.clone(),
                        allow: *allow,
                        deny: *deny,
                        lamport: *lamport,
                    },
                );
            }
        }

        // ── Threads: OR-Set ──
        GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name,
            record_key,
            ..
        } => {
            state.threads.insert(
                *thread_id,
                ThreadState {
                    parent_channel_id: *parent_channel_id,
                    name: name.clone(),
                    record_key: record_key.clone(),
                },
            );
        }

        // ── Thread archived (tombstone) ──
        GovernanceEntry::ThreadArchived {
            thread_id, lamport, ..
        } => {
            if let Some(thread) = state.threads.get(thread_id) {
                // Only track for removal — threads don't have created_lamport
                // so we use the thread's existence as the creation signal.
                let _ = thread; // exists check
                if *lamport > 0 {
                    state.threads.remove(thread_id);
                }
            }
        }

        // ── Events: LWW per event_id ──
        GovernanceEntry::EventCreated {
            event_id,
            name,
            description,
            start_time,
            end_time,
            channel_id,
            lamport,
        } => {
            let existing_lamport = state.events.get(event_id).map(|e| e.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.events.insert(
                    *event_id,
                    EventState {
                        name: name.clone(),
                        description: description.clone(),
                        start_time: *start_time,
                        end_time: *end_time,
                        channel_id: *channel_id,
                        lamport: *lamport,
                    },
                );
            }
        }

        // ── Expressions: OR-Set by expression_id ──
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name,
            kind,
            content_hash,
            inline_data,
            animated,
            tags,
            lamport,
        } => {
            let removed_lamport = state
                .expression_remove_lamports
                .get(expression_id)
                .copied()
                .unwrap_or(0);
            let existing_lamport = state
                .expressions
                .get(expression_id)
                .map(|expr| expr.lamport)
                .unwrap_or(0);
            if *lamport > removed_lamport && *lamport > existing_lamport {
                state.expressions.insert(
                    *expression_id,
                    ExpressionState {
                        name: name.clone(),
                        kind: kind.clone(),
                        content_hash: content_hash.clone(),
                        inline_data: inline_data.clone(),
                        animated: *animated,
                        tags: tags.clone(),
                        lamport: *lamport,
                    },
                );
            }
        }

        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport,
        } => {
            let removed_lamport = state
                .expression_remove_lamports
                .get(expression_id)
                .copied()
                .unwrap_or(0);
            if *lamport > removed_lamport {
                state
                    .expression_remove_lamports
                    .insert(*expression_id, *lamport);
            }
            if let Some(expression) = state.expressions.get(expression_id) {
                if *lamport > expression.lamport {
                    state.expressions.remove(expression_id);
                }
            }
        }

        // ── Event archived (tombstone) ──
        GovernanceEntry::EventArchived {
            event_id, lamport, ..
        } => {
            if let Some(event) = state.events.get(event_id) {
                if *lamport > event.lamport {
                    state.events.remove(event_id);
                }
            }
        }

        // ── Onboarding: LWW ──
        GovernanceEntry::OnboardingConfig {
            enabled,
            mode,
            default_channels,
            questions,
            welcome_message,
            guide_steps,
            lamport,
        } => {
            let existing_lamport = state.onboarding.as_ref().map(|o| o.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.onboarding = Some(OnboardingState {
                    enabled: *enabled,
                    mode: mode.clone(),
                    default_channels: default_channels.clone(),
                    questions: questions.clone(),
                    welcome_message: welcome_message.clone(),
                    guide_steps: guide_steps.clone(),
                    lamport: *lamport,
                });
            }
        }

        GovernanceEntry::WelcomeScreen {
            description,
            channels,
            lamport,
        } => {
            let existing_lamport = state
                .welcome_screen
                .as_ref()
                .map(|w| w.lamport)
                .unwrap_or(0);
            if *lamport > existing_lamport {
                state.welcome_screen = Some(WelcomeScreenState {
                    description: description.clone(),
                    channels: channels.clone(),
                    lamport: *lamport,
                });
            }
        }

        // ── Admin delete (tombstone) ──
        GovernanceEntry::AdminDelete { message_id, .. } => {
            state.admin_deletes.insert(*message_id);
        }

        // ── Segment expansion: tracked for join flow discovery ──
        GovernanceEntry::SegmentAdded {
            segment_index,
            registry_key,
            governance_key,
            slot_range_start,
            slot_range_end,
            ..
        } => {
            // Avoid duplicates — segment_index is unique
            if !state
                .segments
                .iter()
                .any(|s| s.segment_index == *segment_index)
            {
                state.segments.push(SegmentState {
                    segment_index: *segment_index,
                    registry_key: registry_key.clone(),
                    governance_key: governance_key.clone(),
                    slot_range_start: *slot_range_start,
                    slot_range_end: *slot_range_end,
                });
                state.segments.sort_by_key(|s| s.segment_index);
            }
        }

        // ── AutoMod rules: LWW per rule_id ──
        GovernanceEntry::AutoModRule {
            rule_id,
            name,
            enabled,
            trigger_json,
            action,
            lamport,
        } => {
            let existing_lamport = state
                .automod_rules
                .get(rule_id)
                .map(|r| r.lamport)
                .unwrap_or(0);
            if *lamport > existing_lamport {
                state.automod_rules.insert(
                    *rule_id,
                    AutoModRuleState {
                        name: name.clone(),
                        enabled: *enabled,
                        trigger_json: trigger_json.clone(),
                        action: action.clone(),
                        lamport: *lamport,
                    },
                );
            }
        }

        // ── Role archived (tombstone) ──
        GovernanceEntry::RoleArchived { role_id, lamport } => {
            // Only archive if lamport > definition lamport
            if let Some(role) = state.roles.get(role_id) {
                if *lamport > role.lamport {
                    state.roles.remove(role_id);
                    // Also remove all assignments for this role
                    for assignments in state.role_assignments.values_mut() {
                        assignments.remove(role_id);
                    }
                }
            }
        }

        // ── Category updated: LWW per category_id ──
        GovernanceEntry::CategoryUpdated {
            category_id,
            name,
            position,
            lamport,
        } => {
            if let Some(cat) = state.categories.get_mut(category_id) {
                if *lamport > cat.created_lamport {
                    if let Some(n) = name {
                        cat.name.clone_from(n);
                    }
                    if let Some(p) = position {
                        cat.position = *p;
                    }
                }
            }
        }

        // ── Invites: OR-Set with revocation tombstone ──
        GovernanceEntry::InviteCreated {
            invite_id,
            code_hash,
            max_uses,
            expires_at,
            encrypted_secrets,
            lamport,
        } => {
            state.invites.insert(
                *invite_id,
                InviteState {
                    code_hash: code_hash.clone(),
                    max_uses: *max_uses,
                    expires_at: *expires_at,
                    encrypted_secrets: encrypted_secrets.clone(),
                    created_lamport: *lamport,
                },
            );
        }

        GovernanceEntry::InviteRevoked {
            invite_id, lamport, ..
        } => {
            // Only revoke if the revocation lamport > creation lamport
            if let Some(invite) = state.invites.get(invite_id) {
                if *lamport > invite.created_lamport {
                    state.invites.remove(invite_id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rekindle_types::id::{ChannelId, RoleId};

    fn pseudo(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    fn role_id(b: u8) -> RoleId {
        RoleId([b; 16])
    }

    fn channel_id(b: u8) -> ChannelId {
        ChannelId([b; 16])
    }

    #[test]
    fn empty_merge() {
        let state = merge(&[]);
        assert!(state.channels.is_empty());
        assert!(state.roles.is_empty());
        assert!(state.creator.is_none());
    }

    #[test]
    fn genesis_sets_creator() {
        let creator = pseudo(1);
        let entries = vec![GovernanceEntry::CommunityMeta {
            name: Some("Test".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        }];
        let state = merge(&[(creator.clone(), entries)]);
        assert_eq!(state.creator, Some(creator));
        assert_eq!(state.metadata.unwrap().name, "Test");
    }

    #[test]
    fn channel_create_and_archive() {
        let creator = pseudo(1);
        let ch = channel_id(1);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_id(0),
                name: "everyone".into(),
                permissions: rekindle_types::permissions::ALL,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::ChannelCreated {
                channel_id: ch,
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "VLD0:abc".into(),
                category_id: None,
                position: 0,
                lamport: 3,
            },
            GovernanceEntry::ChannelArchived {
                channel_id: ch,
                lamport: 4,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert!(
            state.channels.is_empty(),
            "archived channel should be removed"
        );
    }

    #[test]
    fn channel_archive_with_lower_lamport_ignored() {
        let creator = pseudo(1);
        let ch = channel_id(1);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::ChannelCreated {
                channel_id: ch,
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "VLD0:abc".into(),
                category_id: None,
                position: 0,
                lamport: 5,
            },
            // Archive with lower lamport than create — should be ignored
            GovernanceEntry::ChannelArchived {
                channel_id: ch,
                lamport: 3,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert!(
            state.channels.contains_key(&ch),
            "channel should still exist"
        );
    }

    #[test]
    fn role_lww_highest_lamport_wins() {
        let creator = pseudo(1);
        let rid = role_id(1);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: rid,
                name: "old_name".into(),
                permissions: 0,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::RoleDefinition {
                role_id: rid,
                name: "new_name".into(),
                permissions: 0xFF,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 3,
            },
        ];
        let state = merge(&[(creator, entries)]);
        let role = state.roles.get(&rid).unwrap();
        assert_eq!(role.name, "new_name");
        assert_eq!(role.permissions, 0xFF);
    }

    #[test]
    fn ban_unban_lww() {
        let creator = pseudo(1);
        let target = pseudo(2);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_id(0),
                name: "everyone".into(),
                permissions: rekindle_types::permissions::ALL,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::BanEntry {
                target: target.clone(),
                reason: Some("spam".into()),
                lamport: 3,
            },
            GovernanceEntry::UnbanEntry {
                target: target.clone(),
                lamport: 4,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert!(!state.bans.contains(&target), "unban should reverse ban");
    }

    #[test]
    fn mek_max_register() {
        let creator = pseudo(1);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::MEKGenerationBump {
                generation: 3,
                trigger_departed: PseudonymKey([0xBB; 32]),
                cascade_skipped: vec![],
                lamport: 2,
            },
            GovernanceEntry::MEKGenerationBump {
                generation: 1,
                trigger_departed: PseudonymKey([0xCC; 32]),
                cascade_skipped: vec![],
                lamport: 3,
            },
            GovernanceEntry::MEKGenerationBump {
                generation: 5,
                trigger_departed: PseudonymKey([0xDD; 32]),
                cascade_skipped: vec![],
                lamport: 4,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert_eq!(state.mek_generation, 5);
    }

    #[test]
    fn multi_member_merge_converges() {
        // Two members write entries independently — merge should converge
        let member_a = pseudo(1);
        let member_b = pseudo(2);
        let rid = role_id(0);
        let ch = channel_id(1);

        let a_entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: rid,
                name: "everyone".into(),
                permissions: rekindle_types::permissions::ALL,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::ChannelCreated {
                channel_id: ch,
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "VLD0:abc".into(),
                category_id: None,
                position: 0,
                lamport: 3,
            },
        ];

        let b_entries = vec![GovernanceEntry::MEKGenerationBump {
            generation: 2,
            trigger_departed: PseudonymKey([0xEE; 32]),
            cascade_skipped: vec![],
            lamport: 4,
        }];

        // Order A, B
        let state1 = merge(&[
            (member_a.clone(), a_entries.clone()),
            (member_b.clone(), b_entries.clone()),
        ]);

        // Order B, A — should produce same result
        let state2 = merge(&[(member_b, b_entries), (member_a, a_entries)]);

        assert_eq!(
            state1, state2,
            "CRDT convergence: different subkey order must produce same state"
        );
    }

    #[test]
    fn role_assignment_and_unassignment() {
        let creator = pseudo(1);
        let member = pseudo(2);
        let rid = role_id(5);

        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_id(0),
                name: "everyone".into(),
                permissions: rekindle_types::permissions::ALL,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::RoleAssignment {
                target: member.clone(),
                role_id: rid,
                lamport: 3,
            },
            GovernanceEntry::RoleUnassignment {
                target: member.clone(),
                role_id: rid,
                lamport: 4,
            },
        ];
        let state = merge(&[(creator, entries)]);
        let roles = state.role_assignments.get(&member);
        assert!(
            roles.is_none() || roles.unwrap().is_empty(),
            "role should be unassigned"
        );
    }

    #[test]
    fn timeout_lww() {
        let creator = pseudo(1);
        let target = pseudo(2);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_id(0),
                name: "everyone".into(),
                permissions: rekindle_types::permissions::ALL,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::TimeoutEntry {
                target: target.clone(),
                duration_seconds: 3600,
                reason: None,
                started_at: 1000,
                lamport: 3,
            },
        ];
        let state = merge(&[(creator, entries)]);
        let timeout = state.timeouts.get(&target).unwrap();
        assert_eq!(timeout.duration_seconds, 3600);
        assert!(!timeout.is_expired(1500));
        assert!(timeout.is_expired(5000));
    }

    #[test]
    fn remove_timeout_clears_active_timeout() {
        let creator = pseudo(1);
        let target = pseudo(2);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_id(0),
                name: "everyone".into(),
                permissions: rekindle_types::permissions::ALL,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::TimeoutEntry {
                target: target.clone(),
                duration_seconds: 3600,
                reason: None,
                started_at: 1000,
                lamport: 3,
            },
            GovernanceEntry::RemoveTimeoutEntry {
                target: target.clone(),
                lamport: 4,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert!(!state.timeouts.contains_key(&target));
    }

    #[test]
    fn expression_or_set_remove_with_higher_lamport_wins() {
        let creator = pseudo(1);
        let expression_id = [7_u8; 16];
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_id(0),
                name: "everyone".into(),
                permissions: rekindle_types::permissions::ALL,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::ExpressionAdded {
                expression_id,
                name: "wave".into(),
                kind: "emoji".into(),
                content_hash: "hash-a".into(),
                inline_data: Some(vec![1, 2, 3]),
                animated: false,
                tags: vec![],
                lamport: 3,
            },
            GovernanceEntry::ExpressionRemoved {
                expression_id,
                lamport: 4,
            },
        ];

        let state = merge(&[(creator, entries)]);
        assert!(!state.expressions.contains_key(&expression_id));
    }

    #[test]
    fn expression_or_set_add_after_remove_reappears() {
        let creator = pseudo(1);
        let expression_id = [8_u8; 16];
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_id(0),
                name: "everyone".into(),
                permissions: rekindle_types::permissions::ALL,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                lamport: 2,
            },
            GovernanceEntry::ExpressionRemoved {
                expression_id,
                lamport: 3,
            },
            GovernanceEntry::ExpressionAdded {
                expression_id,
                name: "spark".into(),
                kind: "emoji".into(),
                content_hash: "hash-b".into(),
                inline_data: Some(vec![4, 5, 6]),
                animated: true,
                tags: vec!["fun".into()],
                lamport: 4,
            },
        ];

        let state = merge(&[(creator, entries)]);
        let expression = state
            .expressions
            .get(&expression_id)
            .expect("expression should exist");
        assert_eq!(expression.name, "spark");
        assert!(expression.animated);
    }
}

/// Property-based tests using proptest — verifies CRDT convergence guarantee.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use rekindle_types::id::{ChannelId, RoleId};

    fn arb_pseudonym() -> impl Strategy<Value = PseudonymKey> {
        prop::array::uniform32(any::<u8>()).prop_map(PseudonymKey)
    }

    fn arb_channel_id() -> impl Strategy<Value = ChannelId> {
        prop::array::uniform16(any::<u8>()).prop_map(ChannelId)
    }

    fn arb_role_id() -> impl Strategy<Value = RoleId> {
        prop::array::uniform16(any::<u8>()).prop_map(RoleId)
    }

    fn arb_entry() -> impl Strategy<Value = GovernanceEntry> {
        prop_oneof![
            // CommunityMeta
            (any::<u64>()).prop_map(|lamport| GovernanceEntry::CommunityMeta {
                name: Some("test".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport,
            }),
            // ChannelCreated
            (arb_channel_id(), any::<u64>()).prop_map(|(ch, lamport)| {
                GovernanceEntry::ChannelCreated {
                    channel_id: ch,
                    name: "ch".into(),
                    channel_type: "text".into(),
                    record_key: "k".into(),
                    category_id: None,
                    position: 0,
                    lamport,
                }
            }),
            // RoleDefinition
            (arb_role_id(), any::<u64>(), any::<u64>()).prop_map(|(rid, perms, lamport)| {
                GovernanceEntry::RoleDefinition {
                    role_id: rid,
                    name: "role".into(),
                    permissions: perms,
                    position: 0,
                    color: 0,
                    hoist: false,
                    mentionable: false,
                    self_assignable: false,
                    lamport,
                }
            }),
            // ExpressionAdded
            (prop::array::uniform16(any::<u8>()), any::<u64>()).prop_map(
                |(expression_id, lamport)| {
                    GovernanceEntry::ExpressionAdded {
                        expression_id,
                        name: "emoji_name".into(),
                        kind: "emoji".into(),
                        content_hash: "hash".into(),
                        inline_data: Some(vec![1, 2, 3]),
                        animated: false,
                        tags: vec!["test".into()],
                        lamport,
                    }
                }
            ),
            // ExpressionRemoved
            (prop::array::uniform16(any::<u8>()), any::<u64>()).prop_map(
                |(expression_id, lamport)| {
                    GovernanceEntry::ExpressionRemoved {
                        expression_id,
                        lamport,
                    }
                }
            ),
            // MEKGenerationBump
            (any::<u64>(), any::<u64>(), arb_pseudonym()).prop_map(|(gen, lamport, departed)| {
                GovernanceEntry::MEKGenerationBump {
                    generation: gen,
                    trigger_departed: departed,
                    cascade_skipped: vec![],
                    lamport,
                }
            }),
            // BanEntry
            (arb_pseudonym(), any::<u64>()).prop_map(|(target, lamport)| {
                GovernanceEntry::BanEntry {
                    target,
                    reason: None,
                    lamport,
                }
            }),
        ]
    }

    proptest! {
        /// **CRDT convergence:** Two subkey orderings produce identical state.
        #[test]
        fn merge_is_order_independent(
            author_a in arb_pseudonym(),
            author_b in arb_pseudonym(),
            entries_a in prop::collection::vec(arb_entry(), 1..5),
            entries_b in prop::collection::vec(arb_entry(), 0..3),
        ) {
            let state1 = merge(&[
                (author_a.clone(), entries_a.clone()),
                (author_b.clone(), entries_b.clone()),
            ]);
            let state2 = merge(&[
                (author_b, entries_b),
                (author_a, entries_a),
            ]);
            prop_assert_eq!(state1, state2);
        }

        /// **Idempotence:** Merging the same entries twice doesn't change the state.
        #[test]
        fn merge_is_idempotent(
            author in arb_pseudonym(),
            entries in prop::collection::vec(arb_entry(), 1..5),
        ) {
            let state1 = merge(&[(author.clone(), entries.clone())]);
            let state2 = merge(&[
                (author.clone(), entries.clone()),
                (author, entries),
            ]);
            prop_assert_eq!(state1, state2);
        }
    }
}
