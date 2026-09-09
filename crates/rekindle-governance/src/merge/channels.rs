//! `merge` channels CRDT apply rules.

use super::{CategoryState, ChannelState, GovernanceEntry, GovernanceState, OverwriteState};

pub(super) fn apply_channels(entry: &GovernanceEntry, state: &mut GovernanceState) {
    match entry {
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
            let existing_lamport = state.overwrites.get(&key).map_or(0, |o| o.lamport);
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
                .map_or(0, |s| s.linked_lamport);
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
        _ => unreachable!("apply_channels: unexpected variant"),
    }
}
