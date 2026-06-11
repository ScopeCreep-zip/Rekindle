//! Shared community-member route resolution — the DHT presence-row
//! re-resolve both the gossip overlay (`send_to_one_peer` healing) and
//! the voice transport (`request_peer_route_heal`) use when a peer's
//! cached route blob goes stale. One implementation, one trust gate.

use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_protocol::dht::DHTManager;

pub async fn resolve_member_route(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    peer_pseudonym: &str,
) -> Option<Vec<u8>> {
    // Slot location from the discovered-member rows. `subkey_index`
    // is the RAW SMPL slot — segment records are
    // `DHTSchema::smpl(0, members)`, so there is NO owner-subkey
    // offset — and `segment_index` selects the Plate Gate segment
    // record the slot lives in.
    let cid = community_id.to_string();
    let pk = peer_pseudonym.to_string();
    let (subkey_index, segment_index) = crate::db_helpers::db_call(pool, move |conn| {
        conn.query_row(
            "SELECT subkey_index, segment_index FROM community_members \
             WHERE community_id = ?1 AND pseudonym_key = ?2",
            rusqlite::params![cid, pk],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)),
        )
    })
    .await
    .ok()?;

    let registry_key =
        crate::services::community::segments::segment_descriptors(state, community_id)
            .into_iter()
            .find(|d| d.segment_index == segment_index)
            .map(|d| d.registry_key)?;

    let rc = state_helpers::safe_routing_context(state)?;
    let mgr = DHTManager::new(rc);
    let raw = mgr
        .get_value_fresh(&registry_key, subkey_index)
        .await
        .ok()??;

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
