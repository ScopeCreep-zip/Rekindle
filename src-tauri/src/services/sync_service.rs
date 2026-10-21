use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Start the periodic community sync (→ E3.1). A friend's presence and
/// chain arrive by their watches (plan E2.5, D23): nothing here reads them.
pub async fn start_sync_loop(state: Arc<AppState>, pool: Db, stop: CancellationToken) {
    tracing::info!("sync service started");

    // Start at 10s to give gossip overlay setup a chance to complete before
    // the first rejoin attempt. This ensures peers are discovered via presence
    // scanning before we try to broadcast MemberJoinRequest.
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
    let mut tick_count: u32 = 0;

    // The tick's phase runs outside the `select!` and checks the token
    // between items: it is a series of Veilid calls, which are never
    // dropped mid-flight (plan C4.L1). A phase waiting on a record-pool call
    // is released by the pool's drain at logout (C7.6g).
    while stop.run_until_cancelled(interval.tick()).await.is_some() {
        tick_count += 1;
        // After the first 3 rapid ticks, switch to the normal 30s cadence
        if tick_count == 3 {
            interval = tokio::time::interval(std::time::Duration::from_secs(30));
        }
        if let Err(e) =
            crate::services::sync_communities::sync_communities(&state, &pool, &stop).await
        {
            tracing::warn!(error = %e, "community sync failed");
        }
        if stop.is_cancelled() {
            break;
        }
    }
    tracing::info!("sync service shutting down");
}

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
