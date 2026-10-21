use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Start the periodic sync service.
///
/// Runs in the background, periodically syncing local `SQLite` cache with Veilid DHT.
/// - Pull: Read latest from DHT -> update `SQLite`
/// - Push: Send local changes to DHT
///
/// The pending-message retry, the forgetting of removed friends and the
/// request and invite expiry are the Host's friend maintenance (plan E2.1).
pub async fn start_sync_loop(state: Arc<AppState>, pool: Db, stop: CancellationToken) {
    tracing::info!("sync service started");

    // Start at 10s to give gossip overlay setup a chance to complete before
    // the first rejoin attempt. This ensures peers are discovered via presence
    // scanning before we try to broadcast MemberJoinRequest.
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
    let mut watched_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut first_tick = true;
    let mut tick_count: u32 = 0;

    // The tick's phases run outside the `select!` and check the token
    // between phases and between items: each is a series of Veilid calls,
    // which are never dropped mid-flight (plan C4.L1). A phase waiting on a
    // record-pool call is released by the pool's drain at logout (C7.6g).
    while stop.run_until_cancelled(interval.tick()).await.is_some() {
        tick_count += 1;
        let force_all = tick_count.is_multiple_of(10);
        if let Err(e) = sync_friends(&state, &mut watched_keys, first_tick, force_all, &stop).await
        {
            tracing::warn!(error = %e, "friend sync failed");
        }
        first_tick = false;
        // After the first 3 rapid ticks, switch to the normal 30s cadence
        if tick_count == 3 {
            interval = tokio::time::interval(std::time::Duration::from_secs(30));
        }
        if stop.is_cancelled() {
            break;
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

/// Run a single friend sync immediately (called from auth on login).
/// Avoids waiting 30 seconds for the first periodic tick.
///
/// Phase 22.c-REDO — the orchestrator (iterate friends + register
/// + watch + force-poll subkeys + check stale) lives in
/// `rekindle_presence::sync_friends`. This facade builds the
/// adapter and delegates.
pub async fn sync_friends_now(state: &Arc<AppState>) -> Result<(), String> {
    let mut watched_keys = std::collections::HashSet::new();
    let stop = state_helpers::login_scope_or_closed(state).token();
    sync_friends(state, &mut watched_keys, true, true, &stop).await
}

/// Friend-sync tick. Delegates to the crate orchestrator after
/// constructing the presence adapter.
async fn sync_friends(
    state: &Arc<AppState>,
    watched_keys: &mut std::collections::HashSet<String>,
    first_tick: bool,
    force_all: bool,
    stop: &CancellationToken,
) -> Result<(), String> {
    // The Host's one friend presence adapter (plan E2.1); this phase is
    // deleted by name at E2.5 (D4).
    let Some(host) = state.host.current() else {
        return Ok(());
    };
    // Friend sync is record-pool work only (the friend deps make no sends):
    // at logout the pool's drain releases it (plan C7.6g).
    rekindle_presence::sync_friends(
        host.friend_presence(),
        watched_keys,
        first_tick,
        force_all,
        stop,
    )
    .await;
    Ok(())
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
