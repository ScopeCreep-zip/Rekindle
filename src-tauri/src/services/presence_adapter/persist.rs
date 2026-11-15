//! Phase 21 REDO — SQLite-backed helpers for the community
//! presence adapter.
//!
//! Lifted out of `community_deps.rs` so the trait impl file stays
//! focused on the per-method dispatch. Two operations (caught-up channel
//! posts go through the Host's gated ingest, `ingest_catchup`):
//!
//! - `compute_history_ranges` — `MIN/MAX(lamport_ts) GROUP BY
//!   conversation_id` to advertise the Shared Locker history.
//! - `upsert_discovered_member_rows` — atomic batch upsert of
//!   discovered registry members + delete of banned members.

use std::sync::Arc;

use rekindle_presence::DiscoveredMemberRow;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

pub(super) async fn compute_history_ranges(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
) -> Vec<rekindle_types::presence::HistoryRange> {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return Vec::new();
    };
    let cid = community_id.to_string();
    let rows: Result<Vec<(String, u64, u64)>, tokio_rusqlite::Error> = pool
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT conversation_id, MIN(lamport_ts), MAX(lamport_ts) \
                 FROM messages \
                 WHERE owner_key = ?1 \
                   AND conversation_id IN (SELECT id FROM channels WHERE owner_key = ?1 AND community_id = ?2) \
                   AND conversation_type = 'channel' AND lamport_ts > 0 \
                 GROUP BY conversation_id",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![owner_key, cid], |row| {
                    let channel_id_str: String = row.get(0)?;
                    let oldest: u64 = row.get(1)?;
                    let newest: u64 = row.get(2)?;
                    Ok((channel_id_str, oldest, newest))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await;
    match rows {
        Ok(rows) => rows
            .into_iter()
            .map(|(channel_id_str, oldest, newest)| {
                let bytes = decode_channel_id_bytes(&channel_id_str);
                rekindle_types::presence::HistoryRange {
                    channel_id: rekindle_types::id::ChannelId(bytes),
                    oldest_lamport: oldest,
                    newest_lamport: newest,
                }
            })
            .collect(),
        Err(error) => {
            tracing::trace!(
                community = %community_id,
                %error,
                "failed to compute history ranges — will advertise empty",
            );
            Vec::new()
        }
    }
}

fn decode_channel_id_bytes(channel_id_str: &str) -> [u8; 16] {
    hex::decode(channel_id_str)
        .ok()
        .and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok())
        .unwrap_or_else(|| {
            let mut buf = [0u8; 16];
            let src = channel_id_str.as_bytes();
            let len = src.len().min(16);
            buf[..len].copy_from_slice(&src[..len]);
            buf
        })
}

pub(super) fn upsert_discovered_member_rows(
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    rows: Vec<DiscoveredMemberRow>,
    banned_pseudonyms: Vec<String>,
    joined_at: i64,
) {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return;
    };
    let cid = community_id.to_string();
    crate::db_helpers::db_fire(pool, "persist discovered registry members", move |conn| {
        use rekindle_db::repo::members;
        for banned in &banned_pseudonyms {
            members::delete(conn, &owner_key, &cid, banned)?;
        }
        for row in &rows {
            members::upsert_discovered(conn, &owner_key, &cid, row, joined_at)?;
        }
        Ok(())
    });
}
