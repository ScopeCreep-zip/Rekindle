//! Shared community-member route resolution — the DHT presence-row
//! re-resolve both the gossip overlay (`send_to_one_peer` healing) and
//! the voice transport (`request_peer_route_heal`) use when a peer's
//! cached route blob goes stale. One implementation, one trust gate.

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

pub async fn resolve_member_route(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    peer_pseudonym: &str,
) -> Option<Vec<u8>> {
    // Slot location from the discovered-member rows. `subkey_index`
    // is the RAW SMPL slot — segment records are
    // `DHTSchema::smpl(0, members)`, so there is NO owner-subkey
    // offset — and `segment_index` selects the Plate Gate segment
    // record the slot lives in.
    let owner_key = state_helpers::current_owner_key(state).ok()?;
    let cid = community_id.to_string();
    let pk = peer_pseudonym.to_string();
    let (subkey_index, segment_index) = crate::db_helpers::db_call(pool, move |conn| {
        rekindle_db::repo::members::slot(conn, &owner_key, &cid, &pk)
    })
    .await
    .ok()??;

    let registry_key =
        crate::services::community::segments::segment_descriptors(state, community_id)
            .into_iter()
            .find(|d| d.segment_index == segment_index)
            .map(|d| d.registry_key)?;

    let raw = state_helpers::record_pool(state)
        .ok()?
        .read_once(&registry_key.parse().ok()?, subkey_index, true)
        .await
        .ok()??
        .data()
        .to_vec();

    // Same trust gate as the presence scan (W26 signature + ban +
    // liveness) plus a pseudonym match — the slot index comes from
    // local SQLite, and a re-claimed slot must never hand back
    // another member's route.
    let banned: std::collections::HashSet<String> =
        state_helpers::governance_state(state, community_id)
            .map(|gov| {
                gov.bans
                    .iter()
                    .map(|pseudo| hex::encode(pseudo.0))
                    .collect()
            })
            .unwrap_or_default();
    rekindle_presence::route_for_peer(
        &raw,
        peer_pseudonym,
        &banned,
        rekindle_presence::STALE_HEARTBEAT_SECS,
        rekindle_utils::timestamp_secs(),
    )
}
