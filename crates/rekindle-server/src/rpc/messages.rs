use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{
    ChannelMessageDto, CommunityBroadcast, CommunityRequest, CommunityResponse, ReactionGroupDto,
};
use rusqlite::params;

use crate::server_state::ServerState;

use super::permissions::{check_permission, verify_membership};

// ---------------------------------------------------------------------------
// Message request dispatcher
// ---------------------------------------------------------------------------

pub(super) fn dispatch_message_request(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    request: CommunityRequest,
) -> CommunityResponse {
    match request {
        CommunityRequest::SendMessage {
            channel_id,
            ciphertext,
            mek_generation,
            reply_to_id,
        } => handle_send_message(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            ciphertext,
            mek_generation,
            reply_to_id.as_deref(),
        ),
        CommunityRequest::EditMessage {
            channel_id,
            message_id,
            new_ciphertext,
            mek_generation,
        } => handle_edit_message(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            &message_id,
            new_ciphertext,
            mek_generation,
        ),
        CommunityRequest::DeleteMessage {
            channel_id,
            message_id,
        } => handle_delete_message(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            &message_id,
        ),
        CommunityRequest::GetMessages {
            channel_id,
            before_timestamp,
            limit,
        } => handle_get_messages(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            before_timestamp,
            limit,
        ),
        CommunityRequest::AddReaction {
            channel_id,
            message_id,
            emoji,
        } => super::reactions::handle_add_reaction(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            &message_id,
            &emoji,
        ),
        CommunityRequest::RemoveReaction {
            channel_id,
            message_id,
            emoji,
        } => super::reactions::handle_remove_reaction(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            &message_id,
            &emoji,
        ),
        CommunityRequest::PinMessage {
            channel_id,
            message_id,
        } => super::reactions::handle_pin_message(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            &message_id,
        ),
        CommunityRequest::UnpinMessage {
            channel_id,
            message_id,
        } => super::reactions::handle_unpin_message(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            &message_id,
        ),
        CommunityRequest::GetPins { channel_id } => {
            super::reactions::handle_get_pins(state, community_id, sender_pseudonym, &channel_id)
        }
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Message handlers
// ---------------------------------------------------------------------------

fn handle_send_message(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    ciphertext: Vec<u8>,
    mek_generation: u64,
    reply_to_id: Option<&str>,
) -> CommunityResponse {
    // Check SEND_MESSAGES permission (with channel overwrites)
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
        // Announcement channels require MANAGE_COMMUNITY to post
        let channel = community.find_channel(channel_id);
        if channel.is_some_and(|ch| ch.channel_type == "announcement") {
            if let Err(e) =
                check_permission(community, sender_pseudonym, permissions::MANAGE_COMMUNITY)
            {
                return e;
            }
        }

        if let Some(member) = community.find_member(sender_pseudonym) {
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

        let current_gen = community.mek.generation();
        if mek_generation != current_gen {
            return CommunityResponse::Error {
                code: 409,
                message: format!(
                    "MEK generation mismatch: sent {mek_generation}, current is {current_gen}. Request new MEK."
                ),
            };
        }

        // Check slowmode
        if let Some(ch) = community.find_channel(channel_id) {
            if ch.slowmode_seconds > 0 {
                let is_creator = community.is_creator(sender_pseudonym);
                let has_manage =
                    check_permission(community, sender_pseudonym, permissions::MANAGE_MESSAGES)
                        .is_ok();
                if !is_creator && !has_manage {
                    let cooldown = i64::from(ch.slowmode_seconds);
                    let last_ts = state
                        .slowmode_last_message
                        .read()
                        .get(&(channel_id.to_string(), sender_pseudonym.to_string()))
                        .copied()
                        .unwrap_or(0);
                    let now_check = rekindle_utils::timestamp_secs_i64();
                    if now_check - last_ts < cooldown {
                        let remaining = cooldown - (now_check - last_ts);
                        return CommunityResponse::Error {
                            code: 429,
                            message: format!("slowmode: wait {remaining}s"),
                        };
                    }
                }
            }
        }

        // Rate-limit check (auto-moderation): creators and MANAGE_MESSAGES holders bypass
        {
            let is_creator = community.is_creator(sender_pseudonym);
            let has_manage =
                check_permission(community, sender_pseudonym, permissions::MANAGE_MESSAGES).is_ok();
            if !is_creator && !has_manage {
                let rate_now = rekindle_utils::timestamp_secs_i64();
                if !state
                    .rate_limiter
                    .check_and_record(channel_id, sender_pseudonym, rate_now)
                {
                    return CommunityResponse::Error {
                        code: 429,
                        message: "rate limited \u{2014} too many messages".into(),
                    };
                }
            }
        }
    }

    let now = rekindle_utils::timestamp_secs_i64();
    // Message ID format: "msg_" + 16 hex chars (64 bits of randomness).
    // At 1,000 messages/sec the birthday-bound collision probability stays
    // below 1-in-a-billion for ~190 years. If we add the `uuid` crate later,
    // consider migrating to UUIDv7 (time-ordered, 74-bit random, B-tree friendly).
    let message_id = format!("msg_{}", hex::encode(super::rand_bytes(8)));

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let mek_gen_i64 = i64::try_from(mek_generation).unwrap_or(i64::MAX);
        if let Err(e) = db.execute(
            "INSERT INTO server_messages (community_id, channel_id, message_id, sender_pseudonym, ciphertext, mek_generation, timestamp, reply_to_id) VALUES (?,?,?,?,?,?,?,?)",
            params![community_id, channel_id, message_id, sender_pseudonym, ciphertext, mek_gen_i64, now, reply_to_id],
        ) {
            tracing::error!(error = %e, "failed to store message in DB");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to store message".into(),
            };
        }
    }

    // Update slowmode tracker
    state.slowmode_last_message.write().insert(
        (channel_id.to_string(), sender_pseudonym.to_string()),
        now,
    );

    let now_u64: u64 = now.try_into().unwrap_or(0u64);
    super::broadcast::broadcast_to_members(
        state,
        community_id,
        sender_pseudonym,
        &CommunityBroadcast::NewMessage {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message_id: message_id.clone(),
            sender_pseudonym: sender_pseudonym.to_string(),
            ciphertext,
            mek_generation,
            timestamp: now_u64,
            reply_to_id: reply_to_id.map(str::to_string),
        },
    );

    CommunityResponse::MessageSent { message_id, timestamp: now_u64 }
}

fn handle_edit_message(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    message_id: &str,
    new_ciphertext: Vec<u8>,
    mek_generation: u64,
) -> CommunityResponse {
    // Verify membership and SEND_MESSAGES permission (same as sending)
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

    // Verify the sender owns this message
    let original_sender: Result<String, _> = db.query_row(
        "SELECT sender_pseudonym FROM server_messages WHERE community_id = ? AND message_id = ?",
        params![community_id, message_id],
        |row| row.get(0),
    );

    let Ok(original_sender) = original_sender else {
        return CommunityResponse::Error {
            code: 404,
            message: "message not found".into(),
        };
    };

    if original_sender != sender_pseudonym {
        return CommunityResponse::Error {
            code: 403,
            message: "can only edit your own messages".into(),
        };
    }

    let now = rekindle_utils::timestamp_secs_i64();
    let mek_gen_i64 = i64::try_from(mek_generation).unwrap_or(i64::MAX);

    if let Err(e) = db.execute(
        "UPDATE server_messages SET ciphertext = ?, mek_generation = ?, edited_at = ? WHERE community_id = ? AND message_id = ?",
        params![new_ciphertext, mek_gen_i64, now, community_id, message_id],
    ) {
        tracing::error!(error = %e, "failed to update message in DB");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to edit message".into(),
        };
    }

    drop(db);

    let now_u64: u64 = now.try_into().unwrap_or(0u64);
    super::broadcast::broadcast_to_members(
        state,
        community_id,
        "", // include sender so they see edit confirmation
        &CommunityBroadcast::MessageEdited {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message_id: message_id.to_string(),
            new_ciphertext,
            mek_generation,
            edited_at: now_u64,
        },
    );

    CommunityResponse::Ok
}

fn handle_delete_message(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    message_id: &str,
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
    }

    let db = crate::db_helpers::lock_db(&state.db);

    // Check who sent the original message
    let original_sender: Result<String, _> = db.query_row(
        "SELECT sender_pseudonym FROM server_messages WHERE community_id = ? AND message_id = ?",
        params![community_id, message_id],
        |row| row.get(0),
    );

    let Ok(original_sender) = original_sender else {
        return CommunityResponse::Error {
            code: 404,
            message: "message not found".into(),
        };
    };

    // Can delete own messages, or any message with MANAGE_MESSAGES permission
    if original_sender != sender_pseudonym {
        let hosted = state.hosted.read();
        if let Some(community) = hosted.get(community_id) {
            if let Err(e) =
                check_permission(community, sender_pseudonym, permissions::MANAGE_MESSAGES)
            {
                return e;
            }
        }
    }

    if let Err(e) = db.execute(
        "DELETE FROM server_messages WHERE community_id = ? AND message_id = ?",
        params![community_id, message_id],
    ) {
        tracing::error!(error = %e, "failed to delete message from DB");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to delete message".into(),
        };
    }

    drop(db);

    super::broadcast::broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::MessageDeleted {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            message_id: message_id.to_string(),
        },
    );

    CommunityResponse::Ok
}

fn handle_get_messages(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    before_timestamp: Option<u64>,
    limit: u32,
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
        let edited: Option<i64> = row.get(6)?;
        Ok(ChannelMessageDto {
            message_id: row.get(0)?,
            sender_pseudonym: row.get(1)?,
            ciphertext: row.get(2)?,
            mek_generation: mek_gen.try_into().unwrap_or(0u64),
            timestamp: ts.try_into().unwrap_or(0u64),
            reply_to_id: reply_to,
            edited_at: edited.and_then(|v| u64::try_from(v).ok()),
            reactions: Vec::new(),
        })
    };

    let query_result: Result<Vec<ChannelMessageDto>, _> = if let Some(before) = before_timestamp {
        let before_i64: i64 = before.try_into().unwrap_or(i64::MAX);
        db.prepare(
            "SELECT message_id, sender_pseudonym, ciphertext, mek_generation, timestamp, reply_to_id, edited_at FROM server_messages \
             WHERE community_id = ? AND channel_id = ? AND timestamp < ? \
             ORDER BY timestamp DESC LIMIT ?",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(
                params![community_id, channel_id, before_i64, limit],
                row_mapper,
            )?;
            rows.collect()
        })
    } else {
        db.prepare(
            "SELECT message_id, sender_pseudonym, ciphertext, mek_generation, timestamp, reply_to_id, edited_at FROM server_messages \
             WHERE community_id = ? AND channel_id = ? \
             ORDER BY timestamp DESC LIMIT ?",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![community_id, channel_id, limit], row_mapper)?;
            rows.collect()
        })
    };

    let mut messages = match query_result {
        Ok(msgs) => msgs,
        Err(e) => {
            tracing::error!(error = %e, "failed to query messages from DB");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to query messages".into(),
            };
        }
    };

    // Attach reactions to messages
    let message_ids: Vec<String> = messages.iter().map(|m| m.message_id.clone()).collect();
    if !message_ids.is_empty() {
        let placeholders: String = message_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!(
            "SELECT message_id, emoji, reactor_pseudonym FROM server_reactions \
             WHERE community_id = ? AND channel_id = ? AND message_id IN ({placeholders}) \
             ORDER BY created_at"
        );
        let mut stmt = match db.prepare(&query) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "failed to prepare reactions query");
                messages.reverse();
                return CommunityResponse::Messages { messages };
            }
        };

        // Build params: community_id, channel_id, then each message_id
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(community_id.to_string()));
        param_values.push(Box::new(channel_id.to_string()));
        for mid in &message_ids {
            param_values.push(Box::new(mid.clone()));
        }
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(std::convert::AsRef::as_ref).collect();

        let reaction_rows: Vec<(String, String, String)> = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();

        // Group reactions by (message_id, emoji)
        let mut reaction_map: std::collections::HashMap<
            String,
            std::collections::HashMap<String, Vec<String>>,
        > = std::collections::HashMap::new();
        for (msg_id, emoji, reactor) in reaction_rows {
            reaction_map
                .entry(msg_id)
                .or_default()
                .entry(emoji)
                .or_default()
                .push(reactor);
        }

        for msg in &mut messages {
            if let Some(emoji_map) = reaction_map.remove(&msg.message_id) {
                msg.reactions = emoji_map
                    .into_iter()
                    .map(|(emoji, reactors)| ReactionGroupDto {
                        count: u32::try_from(reactors.len()).unwrap_or(u32::MAX),
                        emoji,
                        reactors,
                    })
                    .collect();
            }
        }
    }

    messages.reverse();
    CommunityResponse::Messages { messages }
}
