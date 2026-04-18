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

        // ── Onboarding: LWW ──

        GovernanceEntry::OnboardingConfig {
            enabled,
            mode,
            welcome_message,
            lamport,
        } => {
            let existing_lamport = state.onboarding.as_ref().map(|o| o.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.onboarding = Some(OnboardingState {
                    enabled: *enabled,
                    mode: mode.clone(),
                    welcome_message: welcome_message.clone(),
                    lamport: *lamport,
                });
            }
        }

        // ── Admin delete (tombstone) ──

        GovernanceEntry::AdminDelete { message_id, .. } => {
            state.admin_deletes.insert(*message_id);
        }

        // ── Segment expansion ──

        GovernanceEntry::SegmentAdded { .. } => {
            // Segment tracking is handled at the record layer, not governance state.
            // The existence of this entry is enough — the join flow reads it
            // to discover additional registry/governance records.
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

        GovernanceEntry::InviteCreated { .. } => {
            // Invite entries are stored in governance for discovery by joiners.
            // No state tracking needed here — the join flow reads raw governance
            // entries to find matching invite by code_hash.
        }

        GovernanceEntry::InviteRevoked { .. } => {
            // Revocation tombstone — join flow checks for revocation when matching invites.
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
        assert!(state.channels.is_empty(), "archived channel should be removed");
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
        assert!(state.channels.contains_key(&ch), "channel should still exist");
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
                lamport: 2,
            },
            GovernanceEntry::MEKGenerationBump {
                generation: 1,
                lamport: 3,
            },
            GovernanceEntry::MEKGenerationBump {
                generation: 5,
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
            lamport: 4,
        }];

        // Order A, B
        let state1 = merge(&[
            (member_a.clone(), a_entries.clone()),
            (member_b.clone(), b_entries.clone()),
        ]);

        // Order B, A — should produce same result
        let state2 = merge(&[
            (member_b, b_entries),
            (member_a, a_entries),
        ]);

        assert_eq!(state1, state2, "CRDT convergence: different subkey order must produce same state");
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
                    lamport,
                }
            }),
            // MEKGenerationBump
            (any::<u64>(), any::<u64>()).prop_map(|(gen, lamport)| {
                GovernanceEntry::MEKGenerationBump {
                    generation: gen,
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
