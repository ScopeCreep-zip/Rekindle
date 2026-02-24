use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{
    ChannelMessageDto, CommunityBroadcast, CommunityRequest, CommunityResponse, ThreadInfoDto,
};
use rusqlite::params;

use crate::audit;
use crate::server_state::ServerState;

use super::broadcast::broadcast_to_members;
use super::permissions::{check_permission, verify_membership};

// ---------------------------------------------------------------------------
// Thread dispatch & handlers
// ---------------------------------------------------------------------------

/// Dispatch thread-related requests to their handler functions.
pub(super) fn dispatch_thread_request(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    request: CommunityRequest,
) -> CommunityResponse {
    match request {
        CommunityRequest::CreateThread {
            channel_id,
            name,
            starter_message_id,
        } => handle_create_thread(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            &name,
            &starter_message_id,
        ),
        CommunityRequest::GetChannelThreads { channel_id } => {
            handle_get_channel_threads(state, community_id, sender_pseudonym, &channel_id)
        }
        CommunityRequest::SendThreadMessage {
            thread_id,
            ciphertext,
            mek_generation,
            reply_to_id,
        } => handle_send_thread_message(
            state,
            community_id,
            sender_pseudonym,
            &thread_id,
            ciphertext,
            mek_generation,
            reply_to_id.as_deref(),
        ),
        CommunityRequest::GetThreadMessages {
            thread_id,
            limit,
            before_timestamp,
        } => handle_get_thread_messages(
            state,
            community_id,
            sender_pseudonym,
            &thread_id,
            limit,
            before_timestamp,
        ),
        CommunityRequest::ArchiveThread { thread_id } => {
            handle_archive_thread(state, community_id, sender_pseudonym, &thread_id)
        }
        CommunityRequest::UnarchiveThread { thread_id } => {
            handle_unarchive_thread(state, community_id, sender_pseudonym, &thread_id)
        }
        _ => unreachable!(),
    }
}

fn handle_create_thread(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    name: &str,
    starter_message_id: &str,
) -> CommunityResponse {
    // Validate membership and SEND_MESSAGES permission on the parent channel
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

        if let Some(member) = community.find_member(sender_pseudonym) {
            let channel = community.find_channel(channel_id);
            let ch_overwrites = channel.map_or(&[][..], |ch| &ch.permission_overwrites);
            let perms = permissions::calculate_permissions(
                &member.role_ids,
                &community.roles,
                ch_overwrites,
                sender_pseudonym,
                member.timeout_until,
            );
            if !permissions::has_permission(perms, permissions::SEND_MESSAGES) {
                return CommunityResponse::Error {
                    code: 403,
                    message: "you do not have permission to send messages in this channel".into(),
                };
            }
        }
    }

    let now = rekindle_utils::timestamp_secs_i64();
    let thread_id = format!("thr_{}", hex::encode(super::rand_bytes(8)));

    let db = crate::db_helpers::lock_db(&state.db);
    if let Err(e) = db.execute(
        "INSERT INTO server_threads (community_id, id, channel_id, name, starter_message_id, creator_pseudonym, created_at, archived, auto_archive_seconds, last_message_at) VALUES (?,?,?,?,?,?,?,0,86400,?)",
        params![community_id, thread_id, channel_id, name, starter_message_id, sender_pseudonym, now, now],
    ) {
        tracing::error!(error = %e, "failed to create thread");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to create thread".into(),
        };
    }
    drop(db);

    let now_u64: u64 = now.try_into().unwrap_or(0);
    let thread_info = ThreadInfoDto {
        id: thread_id.clone(),
        channel_id: channel_id.to_string(),
        name: name.to_string(),
        starter_message_id: starter_message_id.to_string(),
        creator_pseudonym: sender_pseudonym.to_string(),
        created_at: now_u64,
        archived: false,
        auto_archive_seconds: 86400,
        last_message_at: now_u64,
        message_count: 0,
    };

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::CreateThread,
        sender_pseudonym,
        Some(&thread_id),
        Some(name),
    );

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::ThreadCreated {
            community_id: community_id.to_string(),
            thread: thread_info,
        },
    );

    CommunityResponse::ThreadCreated { thread_id }
}

fn handle_get_channel_threads(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        if let Some(community) = hosted.get(community_id) {
            if let Err(e) = verify_membership(community, sender_pseudonym) {
                return e;
            }
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);

    let query_result: Result<Vec<ThreadInfoDto>, _> = db
        .prepare(
            "SELECT t.id, t.channel_id, t.name, t.starter_message_id, t.creator_pseudonym, \
                    t.created_at, t.archived, t.auto_archive_seconds, t.last_message_at, \
                    (SELECT COUNT(*) FROM server_thread_messages m WHERE m.community_id = t.community_id AND m.thread_id = t.id) as msg_count \
             FROM server_threads t \
             WHERE t.community_id = ? AND t.channel_id = ? \
             ORDER BY t.last_message_at DESC",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![community_id, channel_id], |row| {
                let created_at: i64 = row.get(5)?;
                let archived: i64 = row.get(6)?;
                let auto_archive: i64 = row.get(7)?;
                let last_msg_at: i64 = row.get(8)?;
                let msg_count: i64 = row.get(9)?;
                Ok(ThreadInfoDto {
                    id: row.get(0)?,
                    channel_id: row.get(1)?,
                    name: row.get(2)?,
                    starter_message_id: row.get(3)?,
                    creator_pseudonym: row.get(4)?,
                    created_at: created_at.try_into().unwrap_or(0),
                    archived: archived != 0,
                    auto_archive_seconds: u32::try_from(auto_archive).unwrap_or(86400),
                    last_message_at: last_msg_at.try_into().unwrap_or(0),
                    message_count: u32::try_from(msg_count).unwrap_or(0),
                })
            })?;
            rows.collect()
        });

    match query_result {
        Ok(threads) => CommunityResponse::ThreadList { threads },
        Err(e) => {
            tracing::error!(error = %e, "failed to query threads from DB");
            CommunityResponse::Error {
                code: 500,
                message: "failed to query threads".into(),
            }
        }
    }
}

fn handle_send_thread_message(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    thread_id: &str,
    ciphertext: Vec<u8>,
    mek_generation: u64,
    reply_to_id: Option<&str>,
) -> CommunityResponse {
    // Verify membership
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

        // Verify MEK generation
        let current_gen = community.mek.generation();
        if mek_generation != current_gen {
            return CommunityResponse::Error {
                code: 409,
                message: format!(
                    "MEK generation mismatch: sent {mek_generation}, current is {current_gen}. Request new MEK."
                ),
            };
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);

    // Verify thread exists and is not archived
    let thread_row: Result<(String, i64), _> = db.query_row(
        "SELECT channel_id, archived FROM server_threads WHERE community_id = ? AND id = ?",
        params![community_id, thread_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );

    let Ok((_channel_id, archived)) = thread_row else {
        return CommunityResponse::Error {
            code: 404,
            message: "thread not found".into(),
        };
    };

    if archived != 0 {
        return CommunityResponse::Error {
            code: 403,
            message: "thread is archived".into(),
        };
    }

    let now = rekindle_utils::timestamp_secs_i64();
    let message_id = format!("msg_{}", hex::encode(super::rand_bytes(8)));
    let mek_gen_i64 = i64::try_from(mek_generation).unwrap_or(i64::MAX);

    if let Err(e) = db.execute(
        "INSERT INTO server_thread_messages (community_id, thread_id, message_id, sender_pseudonym, ciphertext, mek_generation, timestamp, reply_to_id) VALUES (?,?,?,?,?,?,?,?)",
        params![community_id, thread_id, message_id, sender_pseudonym, ciphertext, mek_gen_i64, now, reply_to_id],
    ) {
        tracing::error!(error = %e, "failed to store thread message");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to store thread message".into(),
        };
    }

    // Update last_message_at on the thread
    let _ = db.execute(
        "UPDATE server_threads SET last_message_at = ? WHERE community_id = ? AND id = ?",
        params![now, community_id, thread_id],
    );
    drop(db);

    let now_u64: u64 = now.try_into().unwrap_or(0);

    broadcast_to_members(
        state,
        community_id,
        sender_pseudonym,
        &CommunityBroadcast::ThreadMessageReceived {
            community_id: community_id.to_string(),
            thread_id: thread_id.to_string(),
            message_id: message_id.clone(),
            sender_pseudonym: sender_pseudonym.to_string(),
            ciphertext,
            mek_generation,
            timestamp: now_u64,
            reply_to_id: reply_to_id.map(str::to_string),
        },
    );

    CommunityResponse::MessageSent {
        message_id,
        timestamp: now_u64,
    }
}

fn handle_get_thread_messages(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    thread_id: &str,
    limit: u32,
    before_timestamp: Option<u64>,
) -> CommunityResponse {
    let limit = limit.min(500);

    {
        let hosted = state.hosted.read();
        if let Some(community) = hosted.get(community_id) {
            if let Err(e) = verify_membership(community, sender_pseudonym) {
                return e;
            }
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);

    let row_mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<ChannelMessageDto> {
        let mek_gen: i64 = row.get(3)?;
        let ts: i64 = row.get(4)?;
        let reply_to: Option<String> = row.get(5)?;
        Ok(ChannelMessageDto {
            message_id: row.get(0)?,
            sender_pseudonym: row.get(1)?,
            ciphertext: row.get(2)?,
            mek_generation: mek_gen.try_into().unwrap_or(0u64),
            timestamp: ts.try_into().unwrap_or(0u64),
            reply_to_id: reply_to,
            edited_at: None,
            reactions: Vec::new(),
        })
    };

    let query_result: Result<Vec<ChannelMessageDto>, _> = if let Some(before) = before_timestamp {
        let before_i64: i64 = before.try_into().unwrap_or(i64::MAX);
        db.prepare(
            "SELECT message_id, sender_pseudonym, ciphertext, mek_generation, timestamp, reply_to_id \
             FROM server_thread_messages \
             WHERE community_id = ? AND thread_id = ? AND timestamp < ? \
             ORDER BY timestamp DESC LIMIT ?",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(
                params![community_id, thread_id, before_i64, limit],
                row_mapper,
            )?;
            rows.collect()
        })
    } else {
        db.prepare(
            "SELECT message_id, sender_pseudonym, ciphertext, mek_generation, timestamp, reply_to_id \
             FROM server_thread_messages \
             WHERE community_id = ? AND thread_id = ? \
             ORDER BY timestamp DESC LIMIT ?",
        )
        .and_then(|mut stmt| {
            let rows =
                stmt.query_map(params![community_id, thread_id, limit], row_mapper)?;
            rows.collect()
        })
    };

    match query_result {
        Ok(mut messages) => {
            messages.reverse();
            CommunityResponse::ThreadMessages { messages }
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to query thread messages from DB");
            CommunityResponse::Error {
                code: 500,
                message: "failed to query thread messages".into(),
            }
        }
    }
}

fn handle_archive_thread(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    thread_id: &str,
) -> CommunityResponse {
    set_thread_archive_state(state, community_id, sender_pseudonym, thread_id, true)
}

fn handle_unarchive_thread(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    thread_id: &str,
) -> CommunityResponse {
    set_thread_archive_state(state, community_id, sender_pseudonym, thread_id, false)
}

/// Shared archive/unarchive logic. Validates that the sender is the thread
/// creator or holds `MANAGE_CHANNELS`, then flips the archived flag, logs an
/// audit entry, and broadcasts the change.
fn set_thread_archive_state(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    thread_id: &str,
    archived: bool,
) -> CommunityResponse {
    // Validate: sender must be thread creator OR have MANAGE_CHANNELS
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

        let db = crate::db_helpers::lock_db(&state.db);
        let creator: Result<String, _> = db.query_row(
            "SELECT creator_pseudonym FROM server_threads WHERE community_id = ? AND id = ?",
            params![community_id, thread_id],
            |row| row.get(0),
        );
        drop(db);

        let Ok(creator) = creator else {
            return CommunityResponse::Error {
                code: 404,
                message: "thread not found".into(),
            };
        };

        if creator != sender_pseudonym {
            if let Err(e) =
                check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS)
            {
                return e;
            }
        }
    }

    let archived_int: i32 = i32::from(archived);
    let label = if archived { "archive" } else { "unarchive" };

    let db = crate::db_helpers::lock_db(&state.db);
    if let Err(e) = db.execute(
        "UPDATE server_threads SET archived = ? WHERE community_id = ? AND id = ?",
        params![archived_int, community_id, thread_id],
    ) {
        tracing::error!(error = %e, "failed to {label} thread");
        return CommunityResponse::Error {
            code: 500,
            message: format!("failed to {label} thread"),
        };
    }
    drop(db);

    let action = if archived {
        audit::AuditAction::ArchiveThread
    } else {
        audit::AuditAction::UnarchiveThread
    };
    audit::log_action(
        state,
        community_id,
        action,
        sender_pseudonym,
        Some(thread_id),
        None,
    );

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::ThreadArchived {
            community_id: community_id.to_string(),
            thread_id: thread_id.to_string(),
            archived,
        },
    );

    CommunityResponse::Ok
}
