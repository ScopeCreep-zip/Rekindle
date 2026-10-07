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

use crate::state::{AppState, MemberProfileSnapshot};
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
            community.member_profiles.insert(key, snapshot);
        }
    }
    if emit_refreshed {
        crate::event_dispatch::emit_membership(
            app_handle,
            rekindle_types::subscription_events::MembershipEvent::MembersRefreshed {
                community: community_id.to_string(),
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
        location: local.location.clone(),
    }
}
