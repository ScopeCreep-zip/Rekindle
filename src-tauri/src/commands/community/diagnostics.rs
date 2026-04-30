use tauri::State;

use crate::state::SharedState;
use crate::state_helpers;

use super::types::GossipDiagnostics;

#[tauri::command]
pub async fn debug_gossip_state(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<GossipDiagnostics, String> {
    let communities = state.communities.read();
    let cs = communities
        .get(&community_id)
        .ok_or("community not found")?;

    let has_route_blob =
        state_helpers::our_route_blob(state.inner()).is_some_and(|b| !b.is_empty());
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
