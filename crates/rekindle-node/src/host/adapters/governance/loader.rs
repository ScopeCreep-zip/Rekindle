//! The warm-cache unlock stage, on both modes (plan E3.1 C1): every joined
//! community from the identity's database and vault into the Host's
//! community store, its governance re-merged from `governance_entries_cache`,
//! so the community answers at once, network or not. The DHT rebuild later
//! replaces the governance with anything newer.
//!
//! The cache holds the lossless merge input, so re-running
//! [`rekindle_governance::merge::merge_with_accepted`] reproduces the state
//! the last session merged (merge is reader-validates; a merged view would
//! not). The keys are the vault's: the derived pseudonym, the slot keypair
//! and seed, the registry keypair; the channel keys are the MEK stage's
//! (`HostGovernance::restore_stored_meks`, run right after this one). A key the vault lacks is left absent
//! and re-acquired from a peer, never minted (that forks the community).

use std::collections::{HashMap, HashSet};

use rekindle_db::repo::communities::load::{
    CategoryRow, ChannelRow, CommunityLoad, EventRsvpRow, RoleRow, SlowmodeRow,
};
use rekindle_db::repo::communities::CommunityRow;
use rekindle_db::repo::members::ProfileRow;
use rekindle_transport::session::CommunityMembership;
use rekindle_vault::typed::community_keys;

use crate::daemon::dispatch::DaemonContext;
use crate::host::community::state::{
    CategoryInfo, ChannelInfo, CommunityState, GossipOverlay, MemberProfileSnapshot, RoleDefinition,
};

/// Load every joined community of `owner` into the store, then project the
/// session's memberships from it.
///
/// # Errors
/// The identity's database cannot be read.
pub(crate) async fn restore(
    ctx: &DaemonContext,
    owner: &str,
    secret: &[u8; 32],
) -> Result<(), String> {
    let db = ctx.db.current().map_err(|e| e.to_string())?;
    let states = load(&db, &ctx.vault, owner, secret).await?;
    let memberships: HashMap<String, CommunityMembership> = states
        .values()
        .map(|state| (state.id.clone(), membership(state)))
        .collect();
    *ctx.communities.map().write() = states;
    if let Some(session) = ctx.session.write().as_mut() {
        session.communities = memberships;
    }
    Ok(())
}

/// Every joined community of `owner` as the store holds it: the rows, the
/// keys, and the cached governance re-merged.
///
/// # Errors
/// The identity's database cannot be read.
pub(crate) async fn load(
    db: &rekindle_db::Db,
    vault: &rekindle_vault::VaultHandle,
    owner: &str,
    secret: &[u8; 32],
) -> Result<HashMap<String, CommunityState>, String> {
    let o = owner.to_string();
    let load = db
        .call(move |conn| rekindle_db::repo::communities::load::load(conn, &o))
        .await
        .map_err(|e| format!("communities: {e}"))?;
    let mut states: HashMap<String, CommunityState> = load
        .communities
        .iter()
        .map(|row| (row.id.clone(), assemble(row, &load)))
        .collect();
    restore_keys(vault, &mut states, secret);
    for (community_id, entries) in &load.governance {
        let Some(state) = states.get_mut(community_id) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        let (governance, accepted_clock) = rekindle_governance::merge::merge_with_accepted(entries);
        state.governance_clock = state.governance_clock.max(accepted_clock);
        sync_my_roles(state, &governance);
        state.governance_state = Some(governance);
    }
    Ok(states)
}

/// Our role ids from the merged role assignments.
fn sync_my_roles(
    state: &mut CommunityState,
    governance: &rekindle_governance::state::GovernanceState,
) {
    let Some(pseudonym) = state
        .my_pseudonym_key
        .as_deref()
        .and_then(|hex_key| hex::decode(hex_key).ok())
        .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
        .map(rekindle_types::id::PseudonymKey)
    else {
        return;
    };
    if let Some(role_ids) = governance.role_assignments.get(&pseudonym) {
        state.my_role_ids = role_ids
            .iter()
            .copied()
            .map(rekindle_types::id::RoleId::to_legacy_u32)
            .collect();
        state.my_role_ids.sort_unstable();
    }
}

/// The session's membership, a projection of the store's entry (plan E3.1:
/// the Host store is authoritative).
pub(crate) fn membership(state: &CommunityState) -> CommunityMembership {
    CommunityMembership {
        governance_key: state.id.clone(),
        pseudonym_key: state.my_pseudonym_key.clone().unwrap_or_default(),
        role_ids: state.my_role_ids.clone(),
        registry_key: state.member_registry_key.clone().unwrap_or_default(),
        slot_index: state.my_subkey_index.unwrap_or(0),
        community_name: state.name.clone(),
        slot_seed: state
            .slot_seed
            .as_deref()
            .and_then(|seed| hex::decode(seed).ok())
            .and_then(|bytes| bytes.try_into().ok()),
        channel_record_keys: state
            .channels
            .iter()
            .filter_map(|c| Some((c.id.clone(), c.message_record_key.clone()?)))
            .collect(),
        segment_index: state.my_segment_index,
        lamport_counter: state.governance_clock,
        mek_generation: state.mek_generation,
    }
}

/// The pseudonym (derived, deterministic from the identity secret and the
/// community id) and the vault's slot and registry keys.
fn restore_keys(
    vault: &rekindle_vault::VaultHandle,
    states: &mut HashMap<String, CommunityState>,
    secret: &[u8; 32],
) {
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
    let guard = vault.lock();
    for (community_id, state) in states.iter_mut() {
        if state.my_pseudonym_key.is_none() {
            let signing = derive_community_pseudonym(secret, community_id);
            state.my_pseudonym_key = Some(hex::encode(signing.verifying_key().as_bytes()));
        }
        let Some(vault) = guard.as_ref() else {
            continue;
        };
        state.slot_keypair = community_keys::load_slot_keypair(vault, community_id);
        state.slot_seed = community_keys::load_slot_seed(vault, community_id);
        state.registry_owner_keypair = community_keys::load_registry_keypair(vault, community_id);
    }
}

fn assemble(community: &CommunityRow, load: &CommunityLoad) -> CommunityState {
    let id = community.id.as_str();
    let (channels, channel_log_keys, channel_sequences) = channels_of(id, &load.channels);
    CommunityState {
        name: community.name.clone(),
        description: community.description.clone(),
        icon_hash: community.icon_hash.clone(),
        banner_hash: community.banner_hash.clone(),
        channels,
        categories: categories_of(id, &load.categories),
        my_role_ids: community.my_role_ids.clone(),
        roles: roles_of(id, &load.roles),
        dht_owner_keypair: community.dht_owner_keypair.clone(),
        my_pseudonym_key: community.my_pseudonym_key.clone(),
        mek_generation: community.mek_generation,
        member_registry_key: community.member_registry_key.clone(),
        my_subkey_index: community.my_subkey_index,
        my_segment_index: community.my_segment_index,
        gossip: Some(GossipOverlay::default()),
        channel_log_keys,
        channel_sequences,
        channel_last_send_at: slowmode_of(id, &load.slowmode),
        known_members: load
            .members
            .iter()
            .filter(|r| r.community_id == id)
            .map(|r| r.pseudonym_key.clone())
            .collect::<HashSet<_>>(),
        my_event_rsvps: rsvps_of(id, &load.event_rsvps),
        onboarding_complete: community.onboarding_complete,
        message_clock: community.message_clock,
        governance_clock: community.governance_clock,
        member_profiles: profiles_of(id, &load.members),
        // NULL column: the community never set a policy.
        presence_policy: community.presence_policy.clone().unwrap_or_default(),
        ..CommunityState::new(id)
    }
}

type Channels = (
    Vec<ChannelInfo>,
    HashMap<String, String>,
    HashMap<String, u64>,
);

/// One community's channels, with their log keys and our sequences.
fn channels_of(community_id: &str, rows: &[ChannelRow]) -> Channels {
    let mut log_keys = HashMap::new();
    let mut sequences = HashMap::new();
    let channels = rows
        .iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| {
            if let Some(log_key) = &r.log_key {
                log_keys.insert(r.id.clone(), log_key.clone());
            }
            if r.my_sequence > 0 {
                sequences.insert(r.id.clone(), r.my_sequence);
            }
            ChannelInfo {
                id: r.id.clone(),
                name: r.name.clone(),
                channel_type: r.channel_type,
                unread_count: 0,
                category_id: r.category_id.clone(),
                topic: r.topic.clone(),
                forum_tags: None,
                stage_speakers: Vec::new(),
                stage_moderator: None,
                slowmode_seconds: r.slowmode_seconds,
                nsfw: r.nsfw,
                message_record_key: r.message_record_key.clone(),
                mek_generation: r.mek_generation,
                notification_level: match r.notification_level {
                    1 => "mentions",
                    2 => "nothing",
                    _ => "all",
                }
                .to_string(),
                notification_sound_ref: r.notification_sound_ref.clone(),
                parent_voice_channel_id: r.parent_voice_channel_id.clone(),
            }
        })
        .collect();
    (channels, log_keys, sequences)
}

fn roles_of(community_id: &str, rows: &[RoleRow]) -> Vec<RoleDefinition> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| RoleDefinition {
            id: r.role_id,
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position,
            hoist: r.hoist,
            mentionable: r.mentionable,
            self_assignable: r.self_assignable,
            exclusion_group: r.exclusion_group.clone(),
        })
        .collect()
}

fn categories_of(community_id: &str, rows: &[CategoryRow]) -> Vec<CategoryInfo> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| CategoryInfo {
            id: r.id.clone(),
            name: r.name.clone(),
            sort_order: r.sort_order,
        })
        .collect()
}

fn profiles_of(community_id: &str, rows: &[ProfileRow]) -> HashMap<String, MemberProfileSnapshot> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| {
            let snapshot = MemberProfileSnapshot {
                display_name: r.display_name.clone(),
                bio: r.bio.clone(),
                pronouns: r.pronouns.clone(),
                theme_color: r.theme_color,
                badges: r.badges.clone(),
                avatar_ref: r.avatar_ref.clone(),
                banner_ref: r.banner_ref.clone(),
                // Live session state, never stored: the next presence scan.
                location: None,
            };
            (r.pseudonym_key.clone(), snapshot)
        })
        .collect()
}

fn rsvps_of(community_id: &str, rows: &[EventRsvpRow]) -> HashMap<String, String> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| (r.event_id.clone(), r.status.clone()))
        .collect()
}

fn slowmode_of(community_id: &str, rows: &[SlowmodeRow]) -> HashMap<String, i64> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| (r.channel_id.clone(), r.last_send_ms))
        .collect()
}
