use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{CommunityBroadcast, CommunityResponse, PinnedMessageDto};
use rusqlite::params;

use crate::audit;
use crate::server_state::ServerState;

use super::broadcast::broadcast_to_members;
use super::permissions::{check_permission, verify_membership};

// ---------------------------------------------------------------------------
// Reactions
// ---------------------------------------------------------------------------

pub(super) fn handle_add_reaction(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    message_id: &str,
    emoji: &str,
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
        // Check ADD_REACTIONS permission
        if let Err(e) = check_permission(community, sender_pseudonym, permissions::ADD_REACTIONS) {
            return e;
        }
    }

    let now = rekindle_utils::timestamp_secs_i64();
    let db = crate::db_helpers::lock_db(&state.db);

    // Verify message exists
    let exists: bool = db
        .query_row(
            "SELECT 1 FROM server_messages WHERE community_id = ? AND message_id = ?",
            params![community_id, message_id],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if !exists {
        return CommunityResponse::Error {
            code: 404,
            message: "message not found".into(),
        };
    }

    if let Err(e) = db.execute(
        "INSERT OR IGNORE INTO server_reactions (community_id, channel_id, message_id, emoji, reactor_pseudonym, created_at) VALUES (?,?,?,?,?,?)",
        params![community_id, channel_id, message_id, emoji, sender_pseudonym, now],
    ) {
        tracing::error!(error = %e, "failed to insert reaction");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to add reaction".into(),
        };
    }

    drop(db);

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::ReactionAdded {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message_id: message_id.to_string(),
            emoji: emoji.to_string(),
            reactor_pseudonym: sender_pseudonym.to_string(),
        },
    );

    CommunityResponse::Ok
}

pub(super) fn handle_remove_reaction(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    message_id: &str,
    emoji: &str,
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

    let deleted = db
        .execute(
            "DELETE FROM server_reactions WHERE community_id = ? AND message_id = ? AND emoji = ? AND reactor_pseudonym = ?",
            params![community_id, message_id, emoji, sender_pseudonym],
        )
        .unwrap_or(0);

    drop(db);

    if deleted > 0 {
        broadcast_to_members(
            state,
            community_id,
            "",
            &CommunityBroadcast::ReactionRemoved {
                community_id: community_id.to_string(),
                channel_id: channel_id.to_string(),
                message_id: message_id.to_string(),
                emoji: emoji.to_string(),
                reactor_pseudonym: sender_pseudonym.to_string(),
            },
        );
    }

    CommunityResponse::Ok
}

// ---------------------------------------------------------------------------
// Pinning
// ---------------------------------------------------------------------------

pub(super) fn handle_pin_message(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    message_id: &str,
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
        if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_MESSAGES)
        {
            return e;
        }
    }

    let now = rekindle_utils::timestamp_secs_i64();
    let db = crate::db_helpers::lock_db(&state.db);

    // Verify message exists
    let exists: bool = db
        .query_row(
            "SELECT 1 FROM server_messages WHERE community_id = ? AND message_id = ? AND channel_id = ?",
            params![community_id, message_id, channel_id],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if !exists {
        return CommunityResponse::Error {
            code: 404,
            message: "message not found".into(),
        };
    }

    if let Err(e) = db.execute(
        "INSERT OR IGNORE INTO server_pins (community_id, channel_id, message_id, pinned_by, pinned_at) VALUES (?,?,?,?,?)",
        params![community_id, channel_id, message_id, sender_pseudonym, now],
    ) {
        tracing::error!(error = %e, "failed to pin message");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to pin message".into(),
        };
    }

    drop(db);

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::MessagePinned {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message_id: message_id.to_string(),
            pinned_by: sender_pseudonym.to_string(),
        },
    );

    audit::log_action(state, community_id, audit::AuditAction::PinMessage, sender_pseudonym, None, Some(message_id));
    CommunityResponse::Ok
}

pub(super) fn handle_unpin_message(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    message_id: &str,
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
        if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_MESSAGES)
        {
            return e;
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);

    let deleted = db
        .execute(
            "DELETE FROM server_pins WHERE community_id = ? AND channel_id = ? AND message_id = ?",
            params![community_id, channel_id, message_id],
        )
        .unwrap_or(0);

    drop(db);

    if deleted > 0 {
        broadcast_to_members(
            state,
            community_id,
            "",
            &CommunityBroadcast::MessageUnpinned {
                community_id: community_id.to_string(),
                channel_id: channel_id.to_string(),
                message_id: message_id.to_string(),
            },
        );
    }

    audit::log_action(state, community_id, audit::AuditAction::UnpinMessage, sender_pseudonym, None, Some(message_id));
    CommunityResponse::Ok
}

pub(super) fn handle_get_pins(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
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

    let pins: Vec<PinnedMessageDto> = {
        let mut stmt = match db.prepare(
            "SELECT message_id, channel_id, pinned_by, pinned_at FROM server_pins \
             WHERE community_id = ? AND channel_id = ? ORDER BY pinned_at DESC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "failed to prepare pins query");
                return CommunityResponse::Error {
                    code: 500,
                    message: "failed to query pins".into(),
                };
            }
        };

        stmt.query_map(params![community_id, channel_id], |row| {
            Ok(PinnedMessageDto {
                message_id: row.get(0)?,
                channel_id: row.get(1)?,
                pinned_by: row.get(2)?,
                pinned_at: row.get::<_, i64>(3).map(i64::cast_unsigned)?,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    };

    CommunityResponse::PinnedMessages { pins }
}
