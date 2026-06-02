//! Architecture §4.2 Strategy 1 — LWW entry compaction for governance subkeys.
//!
//! `write_entry` rewrites an author's *entire* SMPL subkey payload on every
//! governance write, appending the new entry. Without compaction the payload
//! grows monotonically and eventually exceeds Veilid's ~4112 B per-subkey cap
//! for a 255-slot governance record, at which point `set_dht_value` fails with
//! a generic "failed schema validation". This module bounds a subkey to the
//! *current state* its author has produced, not the full history.
//!
//! # Convergence
//! Compaction runs immediately before the subkey is signed and published, so
//! every peer (including the author) merges the *same* compacted payload.
//! Compaction therefore can never cause peers to diverge — it only changes
//! what the single published subkey contributes to the shared merge.
//!
//! # Safety — what may and may not be removed
//! `merge` processes all authors' entries in global `(lamport, author)` order
//! and runs `validate_write` against the running state, so an entry can gate
//! *another* author's entry by mutating permission-relevant state mid-stream.
//! Dropping such an entry could retroactively strip a member's permission and
//! make an entity they authored vanish for everyone. We therefore only compact
//! families that do **not** feed `validate_write`:
//!
//! * Channels / categories / events — their archive entry removes them from
//!   state. Nothing in `validate_write` reads category or event state, and a
//!   channel only gates the narrow forum-thread / segment-link path for a
//!   channel that has since been deleted, whose children are dropped anyway
//!   (matching "delete the channel, delete its threads" semantics).
//! * Invites — `InviteRevoked` removes the invite from state; we keep the
//!   revoke tombstone and drop the superseded `InviteCreated`.
//! * Single-key display state — `CommunityMeta`, `CommunityNotificationDefault`,
//!   `OnboardingConfig`, `WelcomeScreen` — keep only the highest-lamport entry.
//! * `MEKGenerationBump` — Max-Register; keep only the highest generation.
//!
//! Roles, moderation (ban/timeout), expressions, automod, permission overwrites
//! and `CommunityPolicy` (invite-quota gating) all feed `validate_write`, so
//! they are kept verbatim.
//!
//! # Genesis guard
//! `merge` force-accepts the globally-lowest-lamport entry and uses its author
//! as `state.creator`. We never drop the author's own minimum-lamport entry,
//! and we never partially prune a channel/category/event/invite lifecycle that
//! contains that entry — otherwise an LWW collapse could shift the global
//! minimum onto a later-joining member and reassign the creator, or a dangling
//! create could resurrect a deleted entity.

use std::collections::{HashMap, HashSet};

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{CategoryId, ChannelId, EventId};

/// Position of an entry within an entity's create → update → archive lifecycle.
#[derive(Clone, Copy)]
enum Life {
    Create,
    Update,
    Archive,
}

/// Compact one author's own governance entries (architecture §4.2 Strategy 1).
///
/// Input is a single author's accumulated subkey log with the new entry already
/// appended. Output is the bounded log to sign and publish. The merged
/// `GovernanceState` is unchanged for every entity that is currently part of
/// the community (see module docs for the safety argument).
pub fn compact_author_entries(entries: Vec<GovernanceEntry>) -> Vec<GovernanceEntry> {
    if entries.len() <= 1 {
        return entries;
    }

    // The author's lowest-lamport entry is force-kept and its lifecycle group
    // is never pruned (genesis guard, see module docs).
    let genesis_lamport = entries
        .iter()
        .map(GovernanceEntry::lamport)
        .min()
        .unwrap_or(0);

    // Entities whose archive supersedes every create → drop the whole lifecycle.
    let dropped_channels = terminated(&entries, genesis_lamport, channel_life);
    let dropped_categories = terminated(&entries, genesis_lamport, category_life);
    let dropped_events = terminated(&entries, genesis_lamport, event_life);
    let (revoked_invites, revoke_keep) = revoked_invites(&entries, genesis_lamport);

    // Highest-lamport survivor for each single-key LWW family.
    let keep_meta = max_lamport(&entries, |e| {
        matches!(e, GovernanceEntry::CommunityMeta { .. })
    });
    let keep_notif = max_lamport(&entries, |e| {
        matches!(e, GovernanceEntry::CommunityNotificationDefault { .. })
    });
    let keep_onboarding = max_lamport(&entries, |e| {
        matches!(e, GovernanceEntry::OnboardingConfig { .. })
    });
    let keep_welcome = max_lamport(&entries, |e| {
        matches!(e, GovernanceEntry::WelcomeScreen { .. })
    });

    // MEK is Max-Register by generation (tie-break by lamport).
    let mek_keep: Option<(u64, u64)> = entries
        .iter()
        .filter_map(|e| match e {
            GovernanceEntry::MEKGenerationBump {
                generation,
                lamport,
                ..
            } => Some((*generation, *lamport)),
            _ => None,
        })
        .max();

    // Highest-lamport create per *live* event — `EventCreated` is full-replace
    // LWW, so only the latest re-publish matters.
    let mut event_create_max: HashMap<EventId, u64> = HashMap::new();
    for e in &entries {
        if let GovernanceEntry::EventCreated {
            event_id, lamport, ..
        } = e
        {
            let slot = event_create_max.entry(*event_id).or_insert(0);
            if *lamport > *slot {
                *slot = *lamport;
            }
        }
    }

    let mut out: Vec<GovernanceEntry> = entries
        .into_iter()
        .filter(|e| {
            // Genesis guard: the author's lowest-lamport entry is always kept.
            if e.lamport() == genesis_lamport {
                return true;
            }
            match e {
                GovernanceEntry::ChannelCreated { channel_id, .. }
                | GovernanceEntry::ChannelUpdated { channel_id, .. }
                | GovernanceEntry::ChannelArchived { channel_id, .. } => {
                    !dropped_channels.contains(channel_id)
                }
                GovernanceEntry::CategoryCreated { category_id, .. }
                | GovernanceEntry::CategoryUpdated { category_id, .. }
                | GovernanceEntry::CategoryArchived { category_id, .. } => {
                    !dropped_categories.contains(category_id)
                }
                GovernanceEntry::EventCreated {
                    event_id, lamport, ..
                } => {
                    !dropped_events.contains(event_id)
                        && event_create_max.get(event_id) == Some(lamport)
                }
                GovernanceEntry::EventArchived { event_id, .. } => {
                    !dropped_events.contains(event_id)
                }
                GovernanceEntry::InviteCreated { invite_id, .. } => {
                    !revoked_invites.contains(invite_id)
                }
                GovernanceEntry::InviteRevoked {
                    invite_id, lamport, ..
                } => revoke_keep
                    .get(invite_id)
                    .is_none_or(|keep| keep == lamport),
                GovernanceEntry::CommunityMeta { lamport, .. } => Some(*lamport) == keep_meta,
                GovernanceEntry::CommunityNotificationDefault { lamport, .. } => {
                    Some(*lamport) == keep_notif
                }
                GovernanceEntry::OnboardingConfig { lamport, .. } => {
                    Some(*lamport) == keep_onboarding
                }
                GovernanceEntry::WelcomeScreen { lamport, .. } => Some(*lamport) == keep_welcome,
                GovernanceEntry::MEKGenerationBump {
                    generation,
                    lamport,
                    ..
                } => Some((*generation, *lamport)) == mek_keep,
                // Keep-all: roles, moderation, expressions, automod, permission
                // overwrites, segments, attachments, threads, channel-segment
                // links, admin-deletes and community policy all gate validation
                // or are order-sensitive (see module docs).
                _ => true,
            }
        })
        .collect();

    // Deterministic output so the read-back verification in `write_entry`
    // compares byte-stable payloads. `merge` re-sorts globally regardless.
    out.sort_by_key(GovernanceEntry::lamport);
    out
}

/// Ids of a create/update/archive family whose archive supersedes every create
/// (the entity is absent from current state). Ids whose lifecycle includes the
/// genesis entry are excluded so the genesis entry — and its terminator — are
/// never partially pruned.
fn terminated<K, F>(entries: &[GovernanceEntry], genesis_lamport: u64, classify: F) -> HashSet<K>
where
    K: std::hash::Hash + Eq + Copy,
    F: Fn(&GovernanceEntry) -> Option<(K, Life)>,
{
    let mut max_create: HashMap<K, u64> = HashMap::new();
    let mut max_archive: HashMap<K, u64> = HashMap::new();
    let mut genesis_ids: HashSet<K> = HashSet::new();
    for e in entries {
        if let Some((key, life)) = classify(e) {
            let lamport = e.lamport();
            if lamport == genesis_lamport {
                genesis_ids.insert(key);
            }
            match life {
                Life::Create => {
                    let slot = max_create.entry(key).or_insert(0);
                    if lamport > *slot {
                        *slot = lamport;
                    }
                }
                Life::Archive => {
                    let slot = max_archive.entry(key).or_insert(0);
                    if lamport > *slot {
                        *slot = lamport;
                    }
                }
                Life::Update => {}
            }
        }
    }
    max_archive
        .into_iter()
        .filter(|(key, archive)| {
            !genesis_ids.contains(key) && max_create.get(key).is_some_and(|create| archive > create)
        })
        .map(|(key, _)| key)
        .collect()
}

/// Invite ids whose revoke supersedes their create (keep-tombstone), plus the
/// surviving (highest-lamport) revoke entry to retain per id. Genesis-bearing
/// invite lifecycles are excluded so the genesis entry is never pruned.
fn revoked_invites(
    entries: &[GovernanceEntry],
    genesis_lamport: u64,
) -> (HashSet<[u8; 16]>, HashMap<[u8; 16], u64>) {
    let mut max_create: HashMap<[u8; 16], u64> = HashMap::new();
    let mut max_revoke: HashMap<[u8; 16], u64> = HashMap::new();
    let mut genesis_ids: HashSet<[u8; 16]> = HashSet::new();
    for e in entries {
        match e {
            GovernanceEntry::InviteCreated {
                invite_id, lamport, ..
            } => {
                if *lamport == genesis_lamport {
                    genesis_ids.insert(*invite_id);
                }
                let slot = max_create.entry(*invite_id).or_insert(0);
                if *lamport > *slot {
                    *slot = *lamport;
                }
            }
            GovernanceEntry::InviteRevoked { invite_id, lamport } => {
                if *lamport == genesis_lamport {
                    genesis_ids.insert(*invite_id);
                }
                let slot = max_revoke.entry(*invite_id).or_insert(0);
                if *lamport > *slot {
                    *slot = *lamport;
                }
            }
            _ => {}
        }
    }
    let revoked: HashSet<[u8; 16]> = max_revoke
        .iter()
        .filter(|(id, revoke)| {
            !genesis_ids.contains(*id) && max_create.get(*id).is_some_and(|create| *revoke > create)
        })
        .map(|(id, _)| *id)
        .collect();
    let revoke_keep: HashMap<[u8; 16], u64> = revoked
        .iter()
        .filter_map(|id| max_revoke.get(id).map(|revoke| (*id, *revoke)))
        .collect();
    (revoked, revoke_keep)
}

fn max_lamport(
    entries: &[GovernanceEntry],
    pred: impl Fn(&GovernanceEntry) -> bool,
) -> Option<u64> {
    entries
        .iter()
        .filter(|e| pred(e))
        .map(|e| e.lamport())
        .max()
}

fn channel_life(e: &GovernanceEntry) -> Option<(ChannelId, Life)> {
    match e {
        GovernanceEntry::ChannelCreated { channel_id, .. } => Some((*channel_id, Life::Create)),
        GovernanceEntry::ChannelUpdated { channel_id, .. } => Some((*channel_id, Life::Update)),
        GovernanceEntry::ChannelArchived { channel_id, .. } => Some((*channel_id, Life::Archive)),
        _ => None,
    }
}

fn category_life(e: &GovernanceEntry) -> Option<(CategoryId, Life)> {
    match e {
        GovernanceEntry::CategoryCreated { category_id, .. } => Some((*category_id, Life::Create)),
        GovernanceEntry::CategoryUpdated { category_id, .. } => Some((*category_id, Life::Update)),
        GovernanceEntry::CategoryArchived { category_id, .. } => {
            Some((*category_id, Life::Archive))
        }
        _ => None,
    }
}

fn event_life(e: &GovernanceEntry) -> Option<(EventId, Life)> {
    match e {
        GovernanceEntry::EventCreated { event_id, .. } => Some((*event_id, Life::Create)),
        GovernanceEntry::EventArchived { event_id, .. } => Some((*event_id, Life::Archive)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::governance::GovernanceSubkeyPayload;
    use rekindle_types::id::{PseudonymKey, RoleId};

    fn meta(lamport: u64) -> GovernanceEntry {
        GovernanceEntry::CommunityMeta {
            name: Some("Test".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport,
        }
    }

    fn chan_created(id: u8, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::ChannelCreated {
            channel_id: ChannelId([id; 16]),
            name: "general".into(),
            channel_type: "text".into(),
            record_key: "VLD0:channelrecordkeyplaceholderplaceholder".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport,
        }
    }

    fn chan_updated(id: u8, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::ChannelUpdated {
            channel_id: ChannelId([id; 16]),
            name: Some("renamed".into()),
            topic: None,
            forum_tags: None,
            position: None,
            slowmode_seconds: None,
            nsfw: None,
            category_id: None,
            lamport,
        }
    }

    fn chan_archived(id: u8, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::ChannelArchived {
            channel_id: ChannelId([id; 16]),
            lamport,
        }
    }

    fn role_def(id: u8, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::RoleDefinition {
            role_id: RoleId([id; 16]),
            name: "mods".into(),
            permissions: 0,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport,
        }
    }

    fn role_archived(id: u8, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::RoleArchived {
            role_id: RoleId([id; 16]),
            lamport,
        }
    }

    fn mek(generation: u64, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::MEKGenerationBump {
            generation,
            trigger_departed: PseudonymKey([0u8; 32]),
            cascade_skipped: Vec::new(),
            lamport,
        }
    }

    fn invite_created(id: u8, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::InviteCreated {
            invite_id: [id; 16],
            code_hash: "0".repeat(64),
            max_uses: 0,
            expires_at: None,
            secrets_record_key: "VLD0:invitesecretsrecordkeyplaceholderplc".into(),
            lamport,
        }
    }

    fn invite_revoked(id: u8, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::InviteRevoked {
            invite_id: [id; 16],
            lamport,
        }
    }

    fn count<F: Fn(&GovernanceEntry) -> bool>(entries: &[GovernanceEntry], pred: F) -> usize {
        entries.iter().filter(|e| pred(e)).count()
    }

    #[test]
    fn single_entry_is_returned_unchanged() {
        let out = compact_author_entries(vec![meta(1)]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn channel_create_and_archive_drop_both() {
        let out =
            compact_author_entries(vec![meta(1), chan_created(0xAA, 2), chan_archived(0xAA, 3)]);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], GovernanceEntry::CommunityMeta { .. }));
    }

    #[test]
    fn updates_for_dead_channel_are_dropped() {
        let out = compact_author_entries(vec![
            meta(1),
            chan_created(0xAA, 2),
            chan_updated(0xAA, 3),
            chan_archived(0xAA, 4),
        ]);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], GovernanceEntry::CommunityMeta { .. }));
    }

    #[test]
    fn live_channel_with_two_updates_is_kept() {
        let out = compact_author_entries(vec![
            meta(1),
            chan_created(0xAA, 2),
            chan_updated(0xAA, 3),
            chan_updated(0xAA, 4),
        ]);
        assert_eq!(out.len(), 4);
        assert_eq!(
            count(&out, |e| matches!(
                e,
                GovernanceEntry::ChannelUpdated { .. }
            )),
            2
        );
    }

    #[test]
    fn roles_are_kept_verbatim() {
        // Conservative deviation from the §4.2 plan: roles feed `validate_write`
        // (cross-author moderation hierarchy + assignment gating), so a same
        // -author define+archive pair is NOT folded.
        let out = compact_author_entries(vec![meta(1), role_def(0xBB, 2), role_archived(0xBB, 3)]);
        assert_eq!(out.len(), 3);
        assert_eq!(
            count(&out, |e| matches!(
                e,
                GovernanceEntry::RoleDefinition { .. }
            )),
            1
        );
        assert_eq!(
            count(&out, |e| matches!(e, GovernanceEntry::RoleArchived { .. })),
            1
        );
    }

    #[test]
    fn community_meta_collapses_to_latest() {
        let out = compact_author_entries(vec![role_def(0xBB, 1), meta(5), meta(10)]);
        assert_eq!(out.len(), 2);
        let metas: Vec<u64> = out
            .iter()
            .filter_map(|e| match e {
                GovernanceEntry::CommunityMeta { lamport, .. } => Some(*lamport),
                _ => None,
            })
            .collect();
        assert_eq!(metas, vec![10]);
    }

    #[test]
    fn genesis_community_meta_is_preserved_even_when_superseded() {
        // The author's lowest-lamport entry is force-kept (creator genesis),
        // so an LWW collapse can never drop it even though a later meta wins.
        let out = compact_author_entries(vec![meta(1), meta(10)]);
        assert_eq!(out.len(), 2);
        assert_eq!(
            count(&out, |e| matches!(e, GovernanceEntry::CommunityMeta { .. })),
            2
        );
    }

    #[test]
    fn mek_keeps_highest_generation() {
        let out = compact_author_entries(vec![meta(1), mek(1, 5), mek(2, 6), mek(2, 7)]);
        let kept: Vec<(u64, u64)> = out
            .iter()
            .filter_map(|e| match e {
                GovernanceEntry::MEKGenerationBump {
                    generation,
                    lamport,
                    ..
                } => Some((*generation, *lamport)),
                _ => None,
            })
            .collect();
        assert_eq!(kept, vec![(2, 7)]);
    }

    #[test]
    fn invite_create_and_revoke_keeps_only_tombstone() {
        let out = compact_author_entries(vec![
            meta(1),
            invite_created(0xCC, 2),
            invite_revoked(0xCC, 3),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(
            count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
            0
        );
        assert_eq!(
            count(&out, |e| matches!(e, GovernanceEntry::InviteRevoked { .. })),
            1
        );
    }

    #[test]
    fn live_invite_is_kept() {
        let out = compact_author_entries(vec![meta(1), invite_created(0xCC, 2)]);
        assert_eq!(out.len(), 2);
        assert_eq!(
            count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
            1
        );
    }

    /// Regression for the original bug: a creator subkey wedged at the ~4112 B
    /// SMPL cap by three dead channel create+archive pairs. Appending the
    /// `InviteCreated` pointer that overflowed the raw log must now fit once the
    /// dead lifecycles are compacted away.
    #[test]
    fn wedged_subkey_self_heals_under_cap_when_invite_appended() {
        let mut entries = vec![
            meta(1),
            // three dead channel pairs (the ~1.3 KiB of removable weight)
            chan_created(0x10, 2),
            chan_archived(0x10, 3),
            chan_created(0x11, 4),
            chan_archived(0x11, 5),
            chan_created(0x12, 6),
            chan_archived(0x12, 7),
            // current state the creator still authors
            chan_created(0x20, 8),
            chan_created(0x21, 9),
            role_def(0x30, 10),
            role_def(0x31, 11),
            mek(1, 12),
            mek(2, 13),
            chan_updated(0x20, 14),
        ];
        // The write that previously overflowed: an invite pointer entry.
        entries.push(invite_created(0xEE, 15));

        let out = compact_author_entries(entries);

        // All three dead channels (and their archives) are gone.
        assert_eq!(
            count(&out, |e| matches!(
                e,
                GovernanceEntry::ChannelArchived { .. }
            )),
            0
        );
        // The two live channels survive.
        assert_eq!(
            count(&out, |e| matches!(
                e,
                GovernanceEntry::ChannelCreated { .. }
            )),
            2
        );
        // Roles and the live invite are untouched.
        assert_eq!(
            count(&out, |e| matches!(
                e,
                GovernanceEntry::RoleDefinition { .. }
            )),
            2
        );
        assert_eq!(
            count(&out, |e| matches!(e, GovernanceEntry::InviteCreated { .. })),
            1
        );
        // Only the highest MEK generation remains.
        assert_eq!(
            count(&out, |e| matches!(
                e,
                GovernanceEntry::MEKGenerationBump { .. }
            )),
            1
        );

        let payload = GovernanceSubkeyPayload {
            author_pseudonym: PseudonymKey([7u8; 32]),
            entries: out,
            signature: vec![0u8; 64],
        };
        let bytes = serde_json::to_vec(&payload).expect("serialize payload");
        assert!(
            bytes.len() < 4112,
            "compacted payload {} B should fit under the SMPL per-subkey cap",
            bytes.len()
        );
    }
}
