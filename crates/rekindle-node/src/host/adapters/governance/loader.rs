//! The warm-cache unlock stage, on both modes (plan E3.1 C1): every joined
//! community from the identity's database and vault into the Host's
//! community store, its governance re-merged from the stored op set
//! (`governance_ops`) under the pinned `GenesisCert`, so the community
//! answers at once, network or not. The DHT rebuild later replaces the
//! governance with anything newer.
//!
//! The op store holds the signed ops, so verifying and merging them
//! (`rekindle_governance_runtime::op_log`) reproduces the state the last
//! session merged. The keys are the vault's: the derived pseudonym, the
//! slot keypair and seed, the registry keypair; the channel keys are the
//! MEK stage's (`HostGovernance::restore_stored_meks`, run right after
//! this one). A key the vault lacks is left absent and re-acquired from a
//! peer, never minted (that forks the community).

use std::collections::{HashMap, HashSet};

use rekindle_db::repo::communities::load::{
    CategoryRow, ChannelRow, CommunityLoad, EventRsvpRow, RoleRow, SlowmodeRow,
};
use rekindle_db::repo::communities::CommunityRow;
use rekindle_db::repo::governance_ops::StoredOp;
use rekindle_db::repo::members::ProfileRow;
use rekindle_transport::session::CommunityMembership;
use rekindle_types::id::PseudonymKey;
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
    // The verified cards cached on the rows (plan E3.4 S6), ours among
    // them, so every name survives a restart.
    let o = owner.to_owned();
    let profiles = db
        .call(move |conn| rekindle_db::repo::members::load_profiles(conn, &o))
        .await
        .map_err(|e| format!("member cards: {e}"))?;
    ctx.communities.restore_member_cards(
        profiles
            .into_iter()
            .filter(|p| p.display_name.is_some() || p.custom_status.is_some())
            .map(|p| {
                let card = rekindle_governance_runtime::CardText {
                    display_name: p.display_name,
                    custom_status: p.custom_status,
                    game: None,
                };
                (
                    p.community_id,
                    PseudonymKey::from_hex_lossy(&p.pseudonym_key),
                    card,
                )
            }),
    );
    // A community we are leaving is in no session list: its leave took
    // effect locally before the restart (plan E3.2 S7d-keys.8).
    let memberships: HashMap<String, CommunityMembership> = states
        .values()
        .filter(|state| !state.departing)
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
    for (community_id, thread_hex) in &load.held_threads {
        let Some(state) = states.get_mut(community_id) else {
            continue;
        };
        let Some(id) = hex::decode(thread_hex)
            .ok()
            .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        else {
            return Err(format!("held thread {thread_hex:?} is not a thread id"));
        };
        state.held_threads.insert(rekindle_types::id::ThreadId(id));
    }
    let mut ops: HashMap<&str, Vec<StoredOp>> = HashMap::new();
    for (community_id, op) in &load.governance {
        ops.entry(community_id.as_str())
            .or_default()
            .push(op.clone());
    }
    for pinned in &load.pinned {
        let community_id = &pinned.community_id;
        let Some(state) = states.get_mut(community_id) else {
            continue;
        };
        let cert = rekindle_codec::governance_op::genesis::decode_genesis(&pinned.genesis_cert)
            .map_err(|e| format!("community {community_id}'s genesis cert: {e}"))?
            .signed
            .cert;
        let held = super::ops::held_ops(
            ops.remove(community_id.as_str()).unwrap_or_default(),
            &pinned.heads,
        )?;
        let set = rekindle_governance_runtime::op_log::verify_held(community_id, &held);
        let mut governance = rekindle_governance::merge::merge(&cert, &set.ops);
        governance
            .threads
            .retain(|id, _| !state.held_threads.contains(id));
        super::projection::project_my_roles(state, &governance);
        // A community we were removed from stays as read-only history: the
        // unlock stages start none of its loops (plan E3.2 S9 own removal).
        // The remover's name as known when the removal was first merged is
        // kept with the row (A/B R5-2).
        let kept_name = (load.communities.iter())
            .find(|row| row.id == *community_id)
            .and_then(|row| row.removed_by_name.clone());
        state.own_removal = (state.my_pseudonym_key.as_deref())
            .and_then(|me| crate::host::community::OwnRemoval::of(&governance, me))
            .map(|removal| crate::host::community::OwnRemoval {
                by_name: kept_name,
                ..removal
            });
        // Nothing of it starts, so its network side counts as torn down: a
        // later merge that voids the removal restores it.
        state.removal_gate =
            std::sync::Arc::new(tokio::sync::Mutex::new(state.own_removal.is_some()));
        state.governance_state = Some(governance);
        state.genesis_cert = Some(cert);
    }
    Ok(states)
}

/// The session's membership, a projection of the store's entry (plan E3.1:
/// the Host store is authoritative).
pub(crate) fn membership(state: &CommunityState) -> CommunityMembership {
    CommunityMembership {
        anchor: state.id.clone(),
        pseudonym_key: state.my_pseudonym_key.clone().unwrap_or_default(),
        registry_key: state.member_registry_key.clone().unwrap_or_default(),
        slot_index: state.my_subkey_index.unwrap_or(0),
        community_name: state.name.clone(),
        slot_seed: state
            .slot_seed
            .as_deref()
            .and_then(|seed| hex::decode(seed).ok())
            .and_then(|bytes| bytes.try_into().ok()),
        // A channel has no record (plan E3.3 slice 8).
        segment_index: state.my_segment_index,
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
    let channels = channels_of(id, &load.channels);
    CommunityState {
        name: community.name.clone(),
        description: community.description.clone(),
        icon_hash: community.icon_hash.clone(),
        banner_hash: community.banner_hash.clone(),
        channels,
        categories: categories_of(id, &load.categories),
        // Laid down by the projection from the merged role assignments.
        my_role_ids: Vec::new(),
        roles: roles_of(id, &load.roles),
        my_pseudonym_key: community.my_pseudonym_key.clone(),
        my_member_record: community.my_member_record.clone(),
        mek_generation: community.mek_generation,
        member_registry_key: community.member_registry_key.clone(),
        my_subkey_index: community.my_subkey_index,
        my_segment_index: community.my_segment_index,
        gossip: Some(GossipOverlay::default()),
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
        member_profiles: profiles_of(id, &load.members),
        // NULL column: the community never set a policy.
        presence_policy: community.presence_policy.clone().unwrap_or_default(),
        departing: community.departing,
        ..with_own_profile(CommunityState::new(id), community, &load.members)
    }
}

/// `state` with our own profile as our roster row stores it (plan E3.4
/// S6), so a restart keeps it.
fn with_own_profile(
    mut state: CommunityState,
    community: &CommunityRow,
    rows: &[ProfileRow],
) -> CommunityState {
    let ours = rows.iter().find(|r| {
        r.community_id == community.id
            && Some(r.pseudonym_key.as_str()) == community.my_pseudonym_key.as_deref()
    });
    if let Some(row) = ours {
        state.my_bio.clone_from(&row.bio);
        state.my_pronouns.clone_from(&row.pronouns);
        state.my_theme_color = row.theme_color;
        state.my_badges.clone_from(&row.badges);
        state.my_avatar_ref.clone_from(&row.avatar_ref);
        state.my_banner_ref.clone_from(&row.banner_ref);
    }
    state
}

/// One community's channels.
fn channels_of(community_id: &str, rows: &[ChannelRow]) -> Vec<ChannelInfo> {
    rows.iter()
        .filter(|r| r.community_id == community_id)
        .map(|r| ChannelInfo {
            id: r.id.clone(),
            name: r.name.clone(),
            channel_type: r.channel_type,
            category_id: r.category_id.clone(),
            topic: r.topic.clone(),
            forum_tags: None,
            stage_speakers: Vec::new(),
            stage_moderator: None,
            slowmode_seconds: r.slowmode_seconds,
            nsfw: r.nsfw,
            mek_generation: r.mek_generation,
            notification_level: match r.notification_level {
                1 => "mentions",
                2 => "nothing",
                _ => "all",
            }
            .to_string(),
            notification_sound_ref: r.notification_sound_ref.clone(),
            parent_voice_channel_id: r.parent_voice_channel_id.clone(),
        })
        .collect()
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
