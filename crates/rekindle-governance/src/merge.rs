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
            parent_voice_channel_id,
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
                    forum_tags: None,
                    slowmode_seconds: None,
                    nsfw: None,
                    parent_voice_channel_id: *parent_voice_channel_id,
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
            forum_tags,
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
                if forum_tags.is_some() {
                    ch.forum_tags.clone_from(forum_tags);
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
            exclusion_group,
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
                        exclusion_group: exclusion_group.clone(),
                        lamport: *lamport,
                    },
                );
            }
        }

        // ── Role assignments: LWW-Flag per (target, role_id) ──
        // Architecture §19.4 — assigning a role with an exclusion_group
        // also removes the member's other assignments in that group
        // (with lower Lamport, since entries are processed in lamport
        // order). This makes "pronouns" / "region" / "team" pickers
        // mutually exclusive without per-group bookkeeping.
        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        } => {
            if let Some(group) = state
                .roles
                .get(role_id)
                .and_then(|role| role.exclusion_group.clone())
            {
                if let Some(roles) = state.role_assignments.get_mut(target) {
                    roles.retain(|other_id| {
                        if other_id == role_id {
                            return true;
                        }
                        state
                            .roles
                            .get(other_id)
                            .and_then(|other| other.exclusion_group.as_ref())
                            != Some(&group)
                    });
                }
            }
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

        // ── Notification default (architecture §17.1 tier 1): LWW ──
        GovernanceEntry::CommunityNotificationDefault { level, lamport } => {
            let existing_lamport = state
                .notification_default
                .as_ref()
                .map_or(0, |d| d.lamport);
            if *lamport > existing_lamport {
                state.notification_default =
                    Some(crate::state::NotificationDefaultState {
                        level: level.clone(),
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
            thread_type,
            record_key,
            invited,
            forum_tag,
            auto_archive_seconds,
            lamport,
        } => {
            let should_replace = state.threads.get(thread_id).is_none_or(|existing| {
                let existing_has_record = existing.record_key.is_some();
                let next_has_record = record_key.is_some();
                if existing_has_record != next_has_record {
                    return next_has_record;
                }
                if *lamport != existing.created_lamport {
                    return *lamport > existing.created_lamport;
                }
                _author.0 > existing.creator.0
            });

            if should_replace {
                let archived_lamport = state
                    .threads
                    .get(thread_id)
                    .and_then(|existing| existing.archived_lamport)
                    .filter(|archived| *archived > *lamport);
                state.threads.insert(
                    *thread_id,
                    ThreadState {
                        parent_channel_id: *parent_channel_id,
                        name: name.clone(),
                        thread_type: thread_type.clone(),
                        record_key: record_key.clone(),
                        invited: invited.clone(),
                        forum_tag: forum_tag.clone(),
                        auto_archive_seconds: *auto_archive_seconds,
                        creator: _author.clone(),
                        created_lamport: *lamport,
                        archived_lamport,
                    },
                );
            }
        }

        // ── Thread archived (tombstone) ──
        GovernanceEntry::ThreadArchived { thread_id, lamport } => {
            if let Some(thread) = state.threads.get_mut(thread_id) {
                if *lamport > thread.created_lamport
                    && thread.archived_lamport.is_none_or(|current| *lamport > current)
                {
                    thread.archived_lamport = Some(*lamport);
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
            cover_image_ref,
            creator_pseudonym,
            recurrence,
            location,
            status,
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
                        cover_image_ref: cover_image_ref.clone(),
                        creator_pseudonym: creator_pseudonym.clone(),
                        recurrence: recurrence.clone(),
                        location: location.clone(),
                        status: status.unwrap_or(rekindle_types::event::EventStatus::Scheduled),
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
            attachment,
            animated,
            tags,
            sound_meta,
            creator_pseudonym,
            created_at,
            available_to_peers,
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
                        attachment: attachment.clone(),
                        animated: *animated,
                        tags: tags.clone(),
                        sound_meta: sound_meta.clone(),
                        creator_pseudonym: creator_pseudonym.clone(),
                        created_at: *created_at,
                        available_to_peers: available_to_peers.unwrap_or(true),
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

        // ── Plate Gate lazy channel records (architecture §15.4):
        //    LWW per (channel_id, segment_index). First-writer wins on
        //    creation; later messages from the same segment write to the
        //    same record (no migration). ──
        GovernanceEntry::ChannelSegmentLinked {
            channel_id,
            segment_index,
            record_key,
            lamport,
        } => {
            let key = (*channel_id, *segment_index);
            let entry_lamport = *lamport;
            let prev = state
                .channel_segment_records
                .get(&key)
                .map(|s| s.linked_lamport)
                .unwrap_or(0);
            if entry_lamport >= prev {
                state.channel_segment_records.insert(
                    key,
                    crate::state::ChannelSegmentRecord {
                        record_key: record_key.clone(),
                        linked_lamport: entry_lamport,
                    },
                );
            }
        }

        // ── Lost Cargo attachment pin/unpin: LWW per attachment_id ──
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned,
            lamport,
        } => {
            let entry_lamport = *lamport;
            let prev = state
                .attachment_pin_lamports
                .get(attachment_id)
                .copied()
                .unwrap_or(0);
            if entry_lamport >= prev {
                state
                    .attachment_pin_lamports
                    .insert(*attachment_id, entry_lamport);
                if *pinned {
                    state.pinned_attachments.insert(*attachment_id);
                } else {
                    state.pinned_attachments.remove(attachment_id);
                }
            }
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

        // ── Community-wide policy (rules text + raid thresholds) ──
        GovernanceEntry::CommunityPolicy {
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
            lamport,
        } => {
            let existing_lamport = state
                .community_policy
                .as_ref()
                .map(|p| p.lamport)
                .unwrap_or(0);
            if *lamport > existing_lamport {
                state.community_policy = Some(CommunityPolicyState {
                    policy_text: policy_text.clone(),
                    max_joins_per_interval: *max_joins_per_interval,
                    join_interval_seconds: *join_interval_seconds,
                    lamport: *lamport,
                });
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

    use rekindle_types::id::{ChannelId, RoleId, ThreadId};

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
                exclusion_group: None,
                lamport: 2,
            },
            GovernanceEntry::ChannelCreated {
                channel_id: ch,
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "VLD0:abc".into(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
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
                parent_voice_channel_id: None,
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
    fn thread_prefers_populated_record_key() {
        let creator = pseudo(1);
        let thread_id = ThreadId([9; 16]);
        let parent_channel_id = channel_id(1);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::ThreadCreated {
                thread_id,
                parent_channel_id,
                name: "ops".into(),
                thread_type: "public".into(),
                record_key: None,
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 86_400,
                lamport: 3,
            },
            GovernanceEntry::ThreadCreated {
                thread_id,
                parent_channel_id,
                name: "ops".into(),
                thread_type: "public".into(),
                record_key: Some("VLD0:thread".into()),
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 86_400,
                lamport: 4,
            },
        ];

        let state = merge(&[(creator, entries)]);
        assert_eq!(
            state
                .threads
                .get(&thread_id)
                .and_then(|thread| thread.record_key.as_deref()),
            Some("VLD0:thread")
        );
    }

    #[test]
    fn thread_archive_sets_tombstone_without_removing_thread() {
        let creator = pseudo(1);
        let thread_id = ThreadId([7; 16]);
        let parent_channel_id = channel_id(1);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::ThreadCreated {
                thread_id,
                parent_channel_id,
                name: "ops".into(),
                thread_type: "public".into(),
                record_key: Some("VLD0:thread".into()),
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 86_400,
                lamport: 3,
            },
            GovernanceEntry::ThreadArchived {
                thread_id,
                lamport: 5,
            },
        ];

        let state = merge(&[(creator, entries)]);
        let thread = state.threads.get(&thread_id).expect("thread should remain materialized");
        assert_eq!(thread.archived_lamport, Some(5));
    }

    #[test]
    fn thread_archive_with_lower_lamport_is_ignored() {
        let creator = pseudo(1);
        let thread_id = ThreadId([7; 16]);
        let parent_channel_id = channel_id(1);
        let entries = vec![
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            },
            GovernanceEntry::ThreadCreated {
                thread_id,
                parent_channel_id,
                name: "ops".into(),
                thread_type: "public".into(),
                record_key: Some("VLD0:thread".into()),
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 86_400,
                lamport: 5,
            },
            GovernanceEntry::ThreadArchived {
                thread_id,
                lamport: 4,
            },
        ];

        let state = merge(&[(creator, entries)]);
        let thread = state.threads.get(&thread_id).expect("thread should remain materialized");
        assert_eq!(thread.archived_lamport, None);
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
                exclusion_group: None,
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
                exclusion_group: None,
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
                exclusion_group: None,
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
                exclusion_group: None,
                lamport: 2,
            },
            GovernanceEntry::ChannelCreated {
                channel_id: ch,
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "VLD0:abc".into(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
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
                exclusion_group: None,
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
                exclusion_group: None,
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
                exclusion_group: None,
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
                exclusion_group: None,
                lamport: 2,
            },
            GovernanceEntry::ExpressionAdded {
                expression_id,
                name: "wave".into(),
                kind: "emoji".into(),
                content_hash: "hash-a".into(),
                attachment: None,
                animated: false,
                tags: vec![],
                sound_meta: None,
                creator_pseudonym: None,
                created_at: None,
                available_to_peers: Some(true),
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
                exclusion_group: None,
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
                attachment: None,
                animated: true,
                tags: vec!["fun".into()],
                sound_meta: None,
                creator_pseudonym: None,
                created_at: None,
                available_to_peers: Some(true),
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

    #[test]
    fn attachment_pin_lww_pin_then_unpin_then_repin() {
        // Sequential lamports — final state matches the highest-lamport entry.
        let creator = pseudo(1);
        let attachment_id = [9u8; 16];
        let entries = vec![
            GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned: true,
                lamport: 1,
            },
            GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned: false,
                lamport: 2,
            },
            GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned: true,
                lamport: 3,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert!(state.pinned_attachments.contains(&attachment_id));
        assert_eq!(state.attachment_pin_lamports.get(&attachment_id), Some(&3));
    }

    #[test]
    fn attachment_pin_out_of_order_arrival_converges() {
        // Same entries, reverse order — LWW must produce the same final state.
        let creator = pseudo(2);
        let attachment_id = [7u8; 16];
        let entries = vec![
            GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned: true,
                lamport: 5,
            },
            GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned: false,
                lamport: 3,
            },
            GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned: true,
                lamport: 1,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert!(state.pinned_attachments.contains(&attachment_id));
        assert_eq!(state.attachment_pin_lamports.get(&attachment_id), Some(&5));
    }

    #[test]
    fn attachment_unpin_at_highest_lamport_clears() {
        let creator = pseudo(3);
        let attachment_id = [4u8; 16];
        let entries = vec![
            GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned: true,
                lamport: 10,
            },
            GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned: false,
                lamport: 11,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert!(!state.pinned_attachments.contains(&attachment_id));
        assert_eq!(state.attachment_pin_lamports.get(&attachment_id), Some(&11));
    }

    #[test]
    fn attachment_pin_independent_per_attachment() {
        let creator = pseudo(4);
        let id_a = [1u8; 16];
        let id_b = [2u8; 16];
        let entries = vec![
            GovernanceEntry::AttachmentPinned {
                attachment_id: id_a,
                pinned: true,
                lamport: 1,
            },
            GovernanceEntry::AttachmentPinned {
                attachment_id: id_b,
                pinned: false,
                lamport: 1,
            },
            GovernanceEntry::AttachmentPinned {
                attachment_id: id_b,
                pinned: true,
                lamport: 2,
            },
        ];
        let state = merge(&[(creator, entries)]);
        assert!(state.pinned_attachments.contains(&id_a));
        assert!(state.pinned_attachments.contains(&id_b));
    }

    #[test]
    fn exclusion_group_unassigns_prior_role_in_same_group() {
        // Architecture §19.4 — pronouns: assigning "he/him" must remove
        // "she/her" automatically because both share the "pronouns" group.
        let creator = pseudo(7);
        let member = pseudo(8);
        let role_he = role_id(11);
        let role_she = role_id(12);
        let role_unrelated = role_id(13);
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
                exclusion_group: None,
                lamport: 2,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_he,
                name: "he/him".into(),
                permissions: 0,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: true,
                exclusion_group: Some("pronouns".into()),
                lamport: 3,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_she,
                name: "she/her".into(),
                permissions: 0,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: true,
                exclusion_group: Some("pronouns".into()),
                lamport: 4,
            },
            GovernanceEntry::RoleDefinition {
                role_id: role_unrelated,
                name: "early-bird".into(),
                permissions: 0,
                position: 1,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: true,
                exclusion_group: None,
                lamport: 5,
            },
            GovernanceEntry::RoleAssignment {
                target: member.clone(),
                role_id: role_she,
                lamport: 6,
            },
            GovernanceEntry::RoleAssignment {
                target: member.clone(),
                role_id: role_unrelated,
                lamport: 7,
            },
            // Switching pronouns: he/him must replace she/her, but
            // early-bird (no exclusion group) stays assigned.
            GovernanceEntry::RoleAssignment {
                target: member.clone(),
                role_id: role_he,
                lamport: 8,
            },
        ];
        let state = merge(&[(creator, entries)]);
        let assignments = state
            .role_assignments
            .get(&member)
            .expect("member must have assignments");
        assert!(assignments.contains(&role_he), "he/him must be active");
        assert!(
            !assignments.contains(&role_she),
            "she/her must have been auto-unassigned"
        );
        assert!(
            assignments.contains(&role_unrelated),
            "non-grouped roles are unaffected"
        );
    }

    #[test]
    fn community_policy_lww_keeps_highest_lamport() {
        let creator = pseudo(5);
        let entries = vec![
            GovernanceEntry::CommunityPolicy {
                policy_text: Some("v1".into()),
                max_joins_per_interval: 10,
                join_interval_seconds: 300,
                lamport: 1,
            },
            GovernanceEntry::CommunityPolicy {
                policy_text: Some("v2".into()),
                max_joins_per_interval: 30,
                join_interval_seconds: 900,
                lamport: 5,
            },
            GovernanceEntry::CommunityPolicy {
                policy_text: Some("stale".into()),
                max_joins_per_interval: 1,
                join_interval_seconds: 60,
                lamport: 3,
            },
        ];
        let state = merge(&[(creator, entries)]);
        let policy = state.community_policy.expect("policy must be set");
        assert_eq!(policy.lamport, 5);
        assert_eq!(policy.policy_text.as_deref(), Some("v2"));
        assert_eq!(policy.max_joins_per_interval, 30);
        assert_eq!(policy.join_interval_seconds, 900);
    }
}

/// Property-based tests using proptest — verifies CRDT convergence guarantee.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use rekindle_types::id::{ChannelId, RoleId, ThreadId};

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
                    parent_voice_channel_id: None,
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
                    exclusion_group: None,
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
                        attachment: None,
                        animated: false,
                        tags: vec!["test".into()],
                        sound_meta: None,
                        creator_pseudonym: None,
                        created_at: None,
                        available_to_peers: Some(true),
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

        #[test]
        fn thread_versions_converge_to_same_active_thread_state(
            thread_id_bytes in prop::array::uniform16(any::<u8>()),
            parent_channel_bytes in prop::array::uniform16(any::<u8>()),
            create_lamport in 2u64..100_000,
            record_delta in 1u64..16,
            archive_delta in 0u64..16,
        ) {
            let creator = PseudonymKey([1; 32]);
            let thread_id = ThreadId(thread_id_bytes);
            let parent_channel_id = ChannelId(parent_channel_bytes);

            let community_meta = GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            };
            let create_without_record = GovernanceEntry::ThreadCreated {
                thread_id,
                parent_channel_id,
                name: "ops".into(),
                thread_type: "public".into(),
                record_key: None,
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 86_400,
                lamport: create_lamport,
            };
            let create_with_record = GovernanceEntry::ThreadCreated {
                thread_id,
                parent_channel_id,
                name: "ops".into(),
                thread_type: "public".into(),
                record_key: Some("VLD0:thread".into()),
                invited: Vec::new(),
                forum_tag: None,
                auto_archive_seconds: 86_400,
                lamport: create_lamport.saturating_add(record_delta),
            };
            let archive = GovernanceEntry::ThreadArchived {
                thread_id,
                lamport: create_lamport.saturating_add(archive_delta),
            };

            let state1 = merge(&[(
                creator.clone(),
                vec![
                    community_meta.clone(),
                    create_without_record.clone(),
                    create_with_record.clone(),
                    archive.clone(),
                ],
            )]);
            let state2 = merge(&[(
                creator,
                vec![
                    community_meta,
                    archive,
                    create_with_record,
                    create_without_record,
                ],
            )]);

            prop_assert_eq!(&state1.threads, &state2.threads);

            let thread = state1
                .threads
                .get(&thread_id)
                .expect("thread must be materialized");
            prop_assert_eq!(thread.record_key.as_deref(), Some("VLD0:thread"));

            let expected_archived =
                (create_lamport.saturating_add(archive_delta) > thread.created_lamport)
                    .then_some(create_lamport.saturating_add(archive_delta));
            prop_assert_eq!(thread.archived_lamport, expected_archived);
        }

        /// **Plate Gate (architecture §15) — segments converge under any
        /// arrival order.** Each segment is its own join-semilattice; the
        /// merged community state is the product CRDT under coordinate-wise
        /// join (Shapiro 2011 / Almeida 2016 arXiv:1603.01529 §3). The
        /// existing merge implementation appends only when `segment_index`
        /// is new; this property test confirms that K random orderings of
        /// the same entry set produce the same final `state.segments`.
        #[test]
        fn segments_converge_regardless_of_order(
            segment_count in 1u32..=4,
            base_lamport in 100u64..1_000,
        ) {
            let creator = PseudonymKey([1; 32]);
            let community_meta = GovernanceEntry::CommunityMeta {
                name: Some("plate-gate".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            };

            // Each segment uses an independent governance + registry key
            // and a contiguous slot range (255 slots per segment per the
            // architecture's universal SMPL schema).
            let segment_entries: Vec<GovernanceEntry> = (1..=segment_count)
                .map(|idx| GovernanceEntry::SegmentAdded {
                    segment_index: idx,
                    registry_key: format!("REG{idx}"),
                    governance_key: format!("GOV{idx}"),
                    slot_range_start: idx * 255,
                    slot_range_end: idx * 255 + 255,
                    lamport: base_lamport.saturating_add(u64::from(idx)),
                })
                .collect();

            // Two orderings: forward and reverse. CRDT idempotence + commutativity
            // guarantees both produce the same merged state.
            let mut forward = vec![community_meta.clone()];
            forward.extend(segment_entries.iter().cloned());
            let mut reverse = vec![community_meta];
            reverse.extend(segment_entries.iter().rev().cloned());

            let state_forward = merge(&[(creator.clone(), forward)]);
            let state_reverse = merge(&[(creator, reverse)]);

            prop_assert_eq!(&state_forward.segments, &state_reverse.segments);
            // Sorted by segment_index ascending (merge.rs:565).
            for window in state_forward.segments.windows(2) {
                prop_assert!(window[0].segment_index < window[1].segment_index);
            }
            // Each segment_index appears exactly once.
            prop_assert_eq!(state_forward.segments.len() as u32, segment_count);
        }
    }
}
