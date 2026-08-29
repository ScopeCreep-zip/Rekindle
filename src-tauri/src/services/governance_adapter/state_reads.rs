//! Phase 23.D.4 — non-trivial state-read helpers extracted from
//! `deps_impl.rs` so the trait impl stays under the 500-LoC cap.

use rekindle_governance_runtime::{
    ChannelMekSnapshot, CommunityMembership, MekSnapshot, OnlineMemberSnapshot,
};
use tauri::Manager;

use super::GovernanceAdapter;

pub(super) fn community_membership_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
) -> Option<CommunityMembership> {
    let communities = adapter.state.communities.read();
    let cs = communities.get(community_id)?;
    Some(CommunityMembership {
        governance_key: cs.governance_key.clone(),
        member_registry_key: cs.member_registry_key.clone(),
        my_pseudonym_hex: cs.my_pseudonym_key.clone(),
        my_subkey_index: cs.my_subkey_index,
        my_segment_index: cs.my_segment_index,
        slot_keypair: cs.slot_keypair.clone(),
        slot_seed_hex: cs.slot_seed.clone(),
        dht_owner_keypair: cs.dht_owner_keypair.clone(),
        lamport_counter: cs.lamport_counter,
        channel_log_keys: cs.channel_log_keys.clone(),
        channel_ids: cs.channels.iter().map(|c| c.id.clone()).collect(),
        mek_generation: cs.mek_generation,
    })
}

pub(super) fn online_members_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
) -> Vec<OnlineMemberSnapshot> {
    let communities = adapter.state.communities.read();
    communities
        .get(community_id)
        .and_then(|cs| cs.gossip.as_ref())
        .map(|gossip| {
            gossip
                .online_members
                .iter()
                .map(|(pseudonym_hex, member)| OnlineMemberSnapshot {
                    pseudonym_hex: pseudonym_hex.clone(),
                    status: member.status.clone(),
                    route_blob: member.route_blob.clone(),
                    last_seen: member.last_seen,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn load_historical_channel_mek_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) -> Option<MekSnapshot> {
    let cache_hit = adapter
        .state
        .channel_mek_cache
        .lock()
        .get(&(community_id.to_string(), channel_id.to_string()))
        .filter(|mek| mek.generation() == generation)
        .map(|mek| MekSnapshot {
            generation: mek.generation(),
            key_bytes: *mek.as_bytes(),
        });
    if cache_hit.is_some() {
        return cache_hit;
    }
    let keystore: tauri::State<'_, crate::keystore::KeystoreHandle> = adapter.app_handle.state();
    let guard = keystore.lock();
    let ks = guard.as_ref()?;
    let mek =
        crate::keystore::load_channel_mek_generation(ks, community_id, channel_id, generation)?;
    Some(MekSnapshot {
        generation: mek.generation(),
        key_bytes: *mek.as_bytes(),
    })
}

pub(super) fn open_record_keys_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
) -> Vec<String> {
    let communities = adapter.state.communities.read();
    communities
        .get(community_id)
        .map(|cs| {
            cs.open_community_records
                .channel_keys
                .iter()
                .cloned()
                .chain(cs.open_community_records.registry_key.iter().cloned())
                .chain(cs.open_community_records.governance_key.iter().cloned())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn channel_meks_all_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
) -> Vec<ChannelMekSnapshot> {
    adapter
        .state
        .channel_mek_cache
        .lock()
        .iter()
        .filter(|((cid, _), _)| cid == community_id)
        .map(|((_, ch), mek)| ChannelMekSnapshot {
            channel_id: ch.clone(),
            mek: MekSnapshot {
                generation: mek.generation(),
                key_bytes: *mek.as_bytes(),
            },
        })
        .collect()
}

pub(super) fn list_my_active_invite_secret_keys_impl(adapter: &GovernanceAdapter) -> Vec<String> {
    let now = rekindle_utils::timestamp_secs();
    let communities = adapter.state.communities.read();
    let mut keys = Vec::new();
    for c in communities.values() {
        let (Some(my_pk_hex), Some(gov)) = (&c.my_pseudonym_key, &c.governance_state) else {
            continue;
        };
        for invite in gov.invites.values() {
            // Only invites we authored are in our local record store, so
            // re-opening them is an instant local-store hit (→ rehydrate).
            if hex::encode(invite.creator_pseudonym.0) != *my_pk_hex {
                continue;
            }
            if invite.expires_at.is_some_and(|exp| exp <= now) {
                continue;
            }
            if !invite.secrets_record_key.is_empty() {
                keys.push(invite.secrets_record_key.clone());
            }
        }
    }
    keys
}
