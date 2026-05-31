//! Thin AppState-bound helpers for `CommunityPresenceDeps`'s
//! member-state methods. Owns the lock-coordinated AppState writes
//! + the RSVP/profile DB-coupled reads-and-writes the
//! orchestrator's pure helpers can't reach by themselves.
//!
//! The actual MERGE logic (role priority composition, RSVP
//! aggregation, profile diff) lives in `crates/rekindle-presence/src/community/{role_merge,rsvp_aggregate,profile_diff}.rs`
//! per Invariant 7. This module just exposes the AppState reads +
//! the post-merge write.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rekindle_types::id::{PseudonymKey, RoleId};

use crate::channels::CommunityEvent;
use crate::db::DbPool;
use crate::state::{AppState, EventRsvpEntry, MemberProfileSnapshot};
use crate::state_helpers;

// ---------- Phase 21.i-fixup.b — role merge primitives ----------

pub(super) fn read_existing_member_roles(
    state: &Arc<AppState>,
    community_id: &str,
) -> HashMap<String, Vec<u32>> {
    state
        .communities
        .read()
        .get(community_id)
        .map(|cs| cs.member_roles.clone())
        .unwrap_or_default()
}

pub(super) fn read_governance_role_assignments(
    state: &Arc<AppState>,
    community_id: &str,
) -> HashMap<PseudonymKey, HashSet<RoleId>> {
    state_helpers::governance_state(state, community_id)
        .map(|gov| gov.role_assignments.clone())
        .unwrap_or_default()
}

pub(super) fn read_my_role_ids(state: &Arc<AppState>, community_id: &str) -> Vec<u32> {
    state
        .communities
        .read()
        .get(community_id)
        .map_or_else(|| vec![0], |cs| cs.my_role_ids.clone())
}

pub(super) fn apply_member_state_update(
    state: &Arc<AppState>,
    community_id: &str,
    merged_member_roles: HashMap<String, Vec<u32>>,
    known_member_keys: HashSet<String>,
    banned_members: &HashSet<String>,
) {
    let mut communities = state.communities.write();
    let Some(cs) = communities.get_mut(community_id) else {
        return;
    };
    for banned in banned_members {
        cs.known_members.remove(banned);
        cs.member_roles.remove(banned);
        if let Some(ref mut gossip) = cs.gossip {
            gossip.online_members.remove(banned);
            gossip.peers.remove(banned);
        }
    }
    cs.known_members.extend(known_member_keys);
    cs.member_roles = merged_member_roles;
}

// ---------- Phase 21.i-fixup.c — RSVP aggregation primitives ----------

pub(super) async fn load_known_event_ids(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
) -> Vec<String> {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return Vec::new();
    };
    let community_id = community_id.to_string();
    pool.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id FROM community_events WHERE owner_key = ?1 AND community_id = ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner_key, community_id], |row| {
                row.get::<_, String>(0)
            })?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        Ok::<Vec<String>, rusqlite::Error>(rows)
    })
    .await
    .unwrap_or_default()
}

pub(super) fn read_my_event_rsvps(
    state: &Arc<AppState>,
    community_id: &str,
) -> HashMap<String, String> {
    state
        .communities
        .read()
        .get(community_id)
        .map(|c| c.my_event_rsvps.clone())
        .unwrap_or_default()
}

pub(super) fn write_event_rsvps_by_event(
    state: &Arc<AppState>,
    community_id: &str,
    aggregated: HashMap<String, Vec<rekindle_presence::EventRsvpEntry>>,
) {
    let local_aggregated: HashMap<String, Vec<EventRsvpEntry>> = aggregated
        .into_iter()
        .map(|(event_id, entries)| {
            let local_entries = entries
                .into_iter()
                .map(|e| EventRsvpEntry {
                    pseudonym_key: e.pseudonym_key,
                    status: e.status,
                })
                .collect();
            (event_id, local_entries)
        })
        .collect();
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.event_rsvps_by_event = local_aggregated;
    }
}

// ---------- Phase 21.i-fixup.d — profile diff primitives ----------

pub(super) fn read_member_profile_snapshot(
    state: &Arc<AppState>,
    community_id: &str,
) -> HashMap<String, rekindle_presence::MemberProfileSnapshot> {
    state
        .communities
        .read()
        .get(community_id)
        .map(|c| {
            c.member_profiles
                .iter()
                .map(|(k, v)| (k.clone(), snapshot_to_crate(v)))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn apply_member_profile_updates(
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    community_id: &str,
    updates: HashMap<String, rekindle_presence::MemberProfileSnapshot>,
    emit_refreshed: bool,
) {
    {
        let mut communities = state.communities.write();
        let Some(community) = communities.get_mut(community_id) else {
            return;
        };
        for (key, snapshot) in updates {
            community
                .member_profiles
                .insert(key, snapshot_from_crate(snapshot));
        }
    }
    if emit_refreshed {
        crate::event_dispatch::emit_live(
            app_handle,
            "community-event",
            &CommunityEvent::MembersRefreshed {
                community_id: community_id.to_string(),
            },
        );
    }
}

fn snapshot_to_crate(local: &MemberProfileSnapshot) -> rekindle_presence::MemberProfileSnapshot {
    rekindle_presence::MemberProfileSnapshot {
        display_name: local.display_name.clone(),
        bio: local.bio.clone(),
        pronouns: local.pronouns.clone(),
        theme_color: local.theme_color,
        badges: local.badges.clone(),
        avatar_ref: local.avatar_ref.clone(),
        banner_ref: local.banner_ref.clone(),
    }
}

fn snapshot_from_crate(
    snapshot: rekindle_presence::MemberProfileSnapshot,
) -> MemberProfileSnapshot {
    MemberProfileSnapshot {
        display_name: snapshot.display_name,
        bio: snapshot.bio,
        pronouns: snapshot.pronouns,
        theme_color: snapshot.theme_color,
        badges: snapshot.badges,
        avatar_ref: snapshot.avatar_ref,
        banner_ref: snapshot.banner_ref,
    }
}
