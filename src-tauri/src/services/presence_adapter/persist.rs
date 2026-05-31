//! Phase 21 REDO — SQLite-backed helpers for the community
//! presence adapter.
//!
//! Lifted out of `community_deps.rs` so the trait impl file stays
//! focused on the per-method dispatch. Three operations:
//!
//! - `insert_channel_catchup_messages` — batch insert of
//!   newly-fetched SMPL channel-log messages, skipping rows
//!   already stored by `message_id`.
//! - `compute_history_ranges` — `MIN/MAX(lamport_ts) GROUP BY
//!   conversation_id` to advertise the Shared Locker history.
//! - `upsert_discovered_member_rows` — atomic batch upsert of
//!   discovered registry members + delete of banned members.

use std::sync::Arc;

use rekindle_presence::DiscoveredMemberRow;
use rekindle_protocol::dht::community::channel_record::ChannelMessage;

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

pub(super) fn insert_channel_catchup_messages(
    state: &Arc<AppState>,
    pool: &DbPool,
    channel_id: &str,
    messages: Vec<ChannelMessage>,
) {
    let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
    let channel = channel_id.to_string();
    crate::db_helpers::db_fire(pool, "smpl_channel_catchup", move |conn| {
        for msg in &messages {
            let mid = msg.message_id.as_deref().unwrap_or("");
            if mid.is_empty() {
                continue;
            }
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE owner_key=?1 AND message_id=?2)",
                    rusqlite::params![owner_key, mid],
                    |r| r.get(0),
                )
                .unwrap_or(false);
            if exists {
                continue;
            }
            let _ = conn.execute(
                "INSERT OR IGNORE INTO messages \
                 (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, message_id, lamport_ts) \
                 VALUES (?1, ?2, 'channel', ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    owner_key,
                    channel,
                    msg.sender_pseudonym,
                    String::from_utf8_lossy(&msg.ciphertext),
                    msg.timestamp,
                    mid,
                    msg.lamport_ts,
                ],
            );
        }
        Ok(())
    });
}

pub(super) async fn compute_history_ranges(
    state: &Arc<AppState>,
    pool: &DbPool,
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
                 WHERE owner_key = ?1 AND community_id = ?2 AND lamport_ts > 0 \
                 GROUP BY conversation_id",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![owner_key, cid], |row| {
                    let channel_id_str: String = row.get(0)?;
                    let oldest: u64 = row.get(1)?;
                    let newest: u64 = row.get(2)?;
                    Ok((channel_id_str, oldest, newest))
                })?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
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
    pool: &DbPool,
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
        for banned in &banned_pseudonyms {
            conn.execute(
                "DELETE FROM community_members WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                rusqlite::params![owner_key, cid, banned],
            )?;
        }
        for row in &rows {
            conn.execute(
                "INSERT INTO community_members \
                 (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at, \
                  subkey_index, segment_index, bio, pronouns, theme_color, badges, \
                  avatar_ref, banner_ref) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
                 ON CONFLICT(owner_key, community_id, pseudonym_key) DO UPDATE SET \
                   display_name = excluded.display_name, \
                   role_ids = excluded.role_ids, \
                   subkey_index = excluded.subkey_index, \
                   segment_index = excluded.segment_index, \
                   bio = excluded.bio, \
                   pronouns = excluded.pronouns, \
                   theme_color = excluded.theme_color, \
                   badges = excluded.badges, \
                   avatar_ref = excluded.avatar_ref, \
                   banner_ref = excluded.banner_ref",
                rusqlite::params![
                    owner_key,
                    cid,
                    row.pseudonym_key,
                    row.display_name,
                    row.role_ids_json,
                    joined_at,
                    row.subkey_index,
                    row.segment_index,
                    row.bio,
                    row.pronouns,
                    row.theme_color,
                    row.badges_json,
                    row.avatar_ref,
                    row.banner_ref,
                ],
            )?;
        }
        Ok(())
    });
}
