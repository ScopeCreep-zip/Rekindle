use std::sync::Arc;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Ask the mesh for a channel's messages since our newest (a watched
/// channel record changed). The periodic community phase is the Host's
/// (`adapters::governance::community_sync`, plan E3.1 C2b).
pub(super) async fn request_channel_sync(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    channel_id: &str,
) {
    let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
    let ch = channel_id.to_string();
    let last_ts: i64 = match db_call(pool, move |conn| {
        conn.query_row(
            "SELECT COALESCE(MAX(timestamp), 0) FROM messages \
             WHERE owner_key=? AND conversation_id=? AND conversation_type='channel'",
            rusqlite::params![owner_key, ch],
            |r| r.get(0),
        )
    })
    .await
    {
        Ok(ts) => ts,
        Err(e) => {
            tracing::warn!(channel = %channel_id, error = %e, "channel sync not requested: last timestamp unreadable");
            return;
        }
    };

    let sync_req = rekindle_codec::community::envelope::CommunityEnvelope::Control(
        rekindle_codec::community::envelope::ControlPayload::SyncRequest {
            channel_id: channel_id.to_string(),
            since_timestamp: last_ts.cast_unsigned(),
        },
    );
    let _ = crate::services::community::send_to_mesh(state, community_id, &sync_req);

    let now = rekindle_utils::timestamp_secs();
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        cs.pending_syncs.insert(channel_id.to_string(), (now, 1));
    }
}
