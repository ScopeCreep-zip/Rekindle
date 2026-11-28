//! Phase 23.C — community-diagnostics body lifted from
//! `commands/community/diagnostics.rs`. The Tauri command stays a
//! thin delegation; this file hosts the `GossipDiagnostics`
//! aggregation that walks AppState fields. Pure read-only state
//! inspection — no protocol logic per Invariant 7.

use crate::state::SharedState;
use crate::state_helpers;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GossipDiagnostics {
    pub community_id: String,
    pub has_gossip: bool,
    pub gossip_peer_count: usize,
    pub online_member_count: usize,
    pub known_member_count: usize,
    pub message_clock: u64,
    pub has_route_blob: bool,
    pub my_pseudonym_key: Option<String>,
    pub my_subkey_index: Option<u32>,
    pub has_slot_keypair: bool,
    pub has_slot_seed: bool,
    pub has_mek: bool,
    pub anchor: Option<String>,
    pub gossip_peer_keys: Vec<String>,
    pub online_member_keys: Vec<String>,
}

pub fn debug_gossip_state_inner(
    state: &SharedState,
    community_id: String,
) -> Result<GossipDiagnostics, String> {
    // The fan-out targets are the Host's mesh (plan E3.4 S5): the roster's
    // live members by their `M`/6 routes.
    let peer_keys: Vec<String> = state_helpers::host_governance(state)
        .map(|host| {
            host.mesh_peers(&community_id)
                .into_iter()
                .map(|p| p.pseudonym_key)
                .collect()
        })
        .unwrap_or_default();
    let communities = state.communities.read();
    let cs = communities
        .get(&community_id)
        .ok_or("community not found")?;

    let has_route_blob = state_helpers::our_route_blob(state).is_some_and(|b| !b.is_empty());
    let has_mek = state_helpers::current_mek(
        state,
        &community_id,
        rekindle_types::channel_keys::KeyScope::Community,
    )
    .is_some();

    let (has_gossip, online_keys) = if let Some(ref g) = cs.gossip {
        (true, g.online_members.keys().cloned().collect::<Vec<_>>())
    } else {
        (false, vec![])
    };

    Ok(GossipDiagnostics {
        community_id,
        has_gossip,
        gossip_peer_count: peer_keys.len(),
        online_member_count: online_keys.len(),
        known_member_count: cs.known_members.len(),
        message_clock: cs.message_clock,
        has_route_blob,
        my_pseudonym_key: cs.my_pseudonym_key.clone(),
        my_subkey_index: cs.my_subkey_index,
        has_slot_keypair: cs.slot_keypair.is_some(),
        has_slot_seed: cs.slot_seed.is_some(),
        has_mek,
        anchor: cs.anchor.clone(),
        gossip_peer_keys: peer_keys,
        online_member_keys: online_keys,
    })
}
