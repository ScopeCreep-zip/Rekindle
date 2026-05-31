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
    pub needs_initial_sync: bool,
    pub lamport_counter: u64,
    pub has_route_blob: bool,
    pub my_pseudonym_key: Option<String>,
    pub my_subkey_index: Option<u32>,
    pub has_slot_keypair: bool,
    pub has_slot_seed: bool,
    pub has_mek: bool,
    pub governance_key: Option<String>,
    pub gossip_peer_keys: Vec<String>,
    pub online_member_keys: Vec<String>,
}

pub fn debug_gossip_state_inner(
    state: &SharedState,
    community_id: String,
) -> Result<GossipDiagnostics, String> {
    let communities = state.communities.read();
    let cs = communities
        .get(&community_id)
        .ok_or("community not found")?;

    let has_route_blob =
        state_helpers::our_route_blob(state).is_some_and(|b| !b.is_empty());
    let has_mek = state.mek_cache.lock().contains_key(&community_id);

    let (has_gossip, peer_count, online_count, needs_sync, lamport, peer_keys, online_keys) =
        if let Some(ref g) = cs.gossip {
            (
                true,
                g.peers.len(),
                g.online_members.len(),
                g.needs_initial_sync,
                g.lamport_counter,
                g.peers.keys().cloned().collect::<Vec<_>>(),
                g.online_members.keys().cloned().collect::<Vec<_>>(),
            )
        } else {
            (false, 0, 0, true, 0, vec![], vec![])
        };

    Ok(GossipDiagnostics {
        community_id,
        has_gossip,
        gossip_peer_count: peer_count,
        online_member_count: online_count,
        known_member_count: cs.known_members.len(),
        needs_initial_sync: needs_sync,
        lamport_counter: lamport,
        has_route_blob,
        my_pseudonym_key: cs.my_pseudonym_key.clone(),
        my_subkey_index: cs.my_subkey_index,
        has_slot_keypair: cs.slot_keypair.is_some(),
        has_slot_seed: cs.slot_seed.is_some(),
        has_mek,
        governance_key: cs.governance_key.clone(),
        gossip_peer_keys: peer_keys,
        online_member_keys: online_keys,
    })
}
