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
//!   revoke tombstone and drop the superseded `InviteCreated`. We additionally
//!   drop expired invites (`expires_at <= now`) and, past
//!   `MAX_ACTIVE_INVITES_PER_INVITER`, the oldest live invites — see
//!   `invites_to_prune`. These prunes are wall-clock / count driven but stay
//!   convergent because compaction output is published and merged identically
//!   by every peer (the clock only affects what THIS author publishes; the
//!   reader-side `check_active_invites_cap` stays clock-free, counting expired
//!   invites until the author's next write sheds them).
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

use crate::invite_quota::MAX_ACTIVE_INVITES_PER_INVITER;

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
pub fn compact_author_entries(
    entries: Vec<GovernanceEntry>,
    now_secs: u64,
) -> Vec<GovernanceEntry> {
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
    let pruned_invites = invites_to_prune(&entries, &revoked_invites, genesis_lamport, now_secs);

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
                    !revoked_invites.contains(invite_id) && !pruned_invites.contains(invite_id)
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

/// Invite ids to drop from this author's subkey beyond the revoke logic:
///   1. expired invites (`expires_at <= now_secs`) — wall-clock pruned. Safe
///      because compaction output is published and merged identically by every
///      peer (the clock only affects what THIS author publishes, never how
///      peers interpret it — see module "Convergence").
///   2. the oldest live invites beyond `MAX_ACTIVE_INVITES_PER_INVITER` —
///      self-heals an author already over the cap (e.g. minted before the cap
///      was lowered). Keep-newest-N is clock-free.
///
/// The genesis-bearing invite and revoked invites (handled separately) are
/// excluded.
fn invites_to_prune(
    entries: &[GovernanceEntry],
    revoked: &HashSet<[u8; 16]>,
    genesis_lamport: u64,
    now_secs: u64,
) -> HashSet<[u8; 16]> {
    let created: Vec<([u8; 16], u64, Option<u64>)> = entries
        .iter()
        .filter_map(|e| match e {
            GovernanceEntry::InviteCreated {
                invite_id,
                lamport,
                expires_at,
                ..
            } => Some((*invite_id, *lamport, *expires_at)),
            _ => None,
        })
        .collect();

    let mut prune: HashSet<[u8; 16]> = HashSet::new();

    // 1. expired (never the genesis entry).
    for (id, lamport, expires_at) in &created {
        if *lamport != genesis_lamport && expires_at.is_some_and(|exp| exp <= now_secs) {
            prune.insert(*id);
        }
    }

    // 2. keep-newest-N among the still-live (not-expired, not-revoked, not-genesis).
    let mut live: Vec<([u8; 16], u64)> = created
        .iter()
        .filter(|(id, lamport, _)| {
            *lamport != genesis_lamport && !prune.contains(id) && !revoked.contains(id)
        })
        .map(|(id, lamport, _)| (*id, *lamport))
        .collect();
    live.sort_by_key(|(_, lamport)| std::cmp::Reverse(*lamport));
    for (id, _) in live
        .into_iter()
        .skip(MAX_ACTIVE_INVITES_PER_INVITER as usize)
    {
        prune.insert(id);
    }

    prune
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
#[path = "compact/tests.rs"]
mod tests;
