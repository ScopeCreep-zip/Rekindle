use std::sync::Arc;

use rekindle_protocol::messaging::envelope::{CommunityResponse, UnreadCountDto};
use rusqlite::params;

use crate::server_state::ServerState;

use super::permissions::verify_membership;

// ---------------------------------------------------------------------------
// Unread tracking
// ---------------------------------------------------------------------------

/// UPSERT the sender's read position for a channel.
pub(super) fn handle_mark_channel_read(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    last_message_id: &str,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);

    // Look up the timestamp for the given message_id so we store it alongside.
    let msg_timestamp: i64 = match db.query_row(
        "SELECT timestamp FROM server_messages WHERE community_id = ? AND message_id = ?",
        params![community_id, last_message_id],
        |row| row.get(0),
    ) {
        Ok(ts) => ts,
        Err(_) => {
            // Message not found — use current time as a reasonable fallback
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .min(u128::from(i64::MAX.cast_unsigned())),
            )
            .unwrap_or(i64::MAX)
        }
    };

    if let Err(e) = db.execute(
        "INSERT INTO server_read_positions (community_id, channel_id, pseudonym_key_hex, last_read_message_id, last_read_timestamp)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(community_id, channel_id, pseudonym_key_hex)
         DO UPDATE SET last_read_message_id = excluded.last_read_message_id,
                       last_read_timestamp = excluded.last_read_timestamp",
        params![community_id, channel_id, sender_pseudonym, last_message_id, msg_timestamp],
    ) {
        tracing::error!(error = %e, "failed to upsert read position");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to update read position".into(),
        };
    }

    CommunityResponse::Ok
}

/// Return unread counts for every channel in the community.
pub(super) fn handle_get_unread_counts(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
) -> CommunityResponse {
    let channel_ids: Vec<String> = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
        community.channels.iter().map(|ch| ch.id.clone()).collect()
    };

    let db = crate::db_helpers::lock_db(&state.db);

    let mut counts = Vec::with_capacity(channel_ids.len());
    for channel_id in &channel_ids {
        // Get the sender's read position timestamp for this channel (if any).
        let last_read_ts: Option<i64> = db
            .query_row(
                "SELECT last_read_timestamp FROM server_read_positions
                 WHERE community_id = ? AND channel_id = ? AND pseudonym_key_hex = ?",
                params![community_id, channel_id, sender_pseudonym],
                |row| row.get(0),
            )
            .ok();

        let unread: u32 = match last_read_ts {
            Some(ts) => {
                // Count messages newer than the read position
                db.query_row(
                    "SELECT COUNT(*) FROM server_messages
                     WHERE community_id = ? AND channel_id = ? AND timestamp > ?",
                    params![community_id, channel_id, ts],
                    |row| row.get(0),
                )
                .unwrap_or(0)
            }
            None => {
                // No read position — all messages are unread
                db.query_row(
                    "SELECT COUNT(*) FROM server_messages
                     WHERE community_id = ? AND channel_id = ?",
                    params![community_id, channel_id],
                    |row| row.get(0),
                )
                .unwrap_or(0)
            }
        };

        counts.push(UnreadCountDto {
            channel_id: channel_id.clone(),
            unread_count: unread,
        });
    }

    CommunityResponse::UnreadCounts { counts }
}
