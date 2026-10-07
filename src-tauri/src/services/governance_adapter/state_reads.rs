//! Phase 23.D.4 — non-trivial state-read helpers extracted from
//! `deps_impl.rs` so the trait impl stays under the 500-LoC cap.

use rekindle_governance_runtime::{CommunityMembership, OnlineMemberSnapshot};

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
        governance_clock: cs.governance_clock,
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
