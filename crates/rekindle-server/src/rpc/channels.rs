use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{
    CategoryDto, ChannelInfoDto, CommunityBroadcast, CommunityRequest, CommunityResponse,
};
use rusqlite::params;

use crate::audit;
use crate::community_host;
use crate::server_state::{ServerCategory, ServerChannel, ServerState};

use super::broadcast::broadcast_to_members;
use super::permissions::{check_permission, verify_membership};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Dispatch channel & category operations, spawning DHT publish on success.
pub(super) fn dispatch_channel_request(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    request: CommunityRequest,
) -> CommunityResponse {
    let resp = match request {
        CommunityRequest::CreateChannel {
            name,
            channel_type,
            category_id,
        } => handle_create_channel(
            state,
            community_id,
            sender_pseudonym,
            &name,
            &channel_type,
            category_id.as_deref(),
        ),
        CommunityRequest::DeleteChannel { channel_id } => {
            handle_delete_channel(state, community_id, sender_pseudonym, &channel_id)
        }
        CommunityRequest::RenameChannel {
            channel_id,
            new_name,
        } => handle_rename_channel(state, community_id, sender_pseudonym, &channel_id, &new_name),
        CommunityRequest::CreateCategory { name } => {
            handle_create_category(state, community_id, sender_pseudonym, &name)
        }
        CommunityRequest::DeleteCategory { category_id } => {
            handle_delete_category(state, community_id, sender_pseudonym, &category_id)
        }
        CommunityRequest::RenameCategory {
            category_id,
            new_name,
        } => handle_rename_category(
            state,
            community_id,
            sender_pseudonym,
            &category_id,
            &new_name,
        ),
        CommunityRequest::MoveChannel {
            channel_id,
            category_id,
        } => handle_move_channel(
            state,
            community_id,
            sender_pseudonym,
            &channel_id,
            category_id.as_deref(),
        ),
        CommunityRequest::ReorderCategories { category_ids } => {
            handle_reorder_categories(state, community_id, sender_pseudonym, &category_ids)
        }
        CommunityRequest::SetChannelTopic { channel_id, topic } => {
            handle_set_channel_topic(state, community_id, sender_pseudonym, &channel_id, &topic)
        }
        CommunityRequest::ReorderChannels { channel_ids } => {
            handle_reorder_channels(state, community_id, sender_pseudonym, &channel_ids)
        }
        CommunityRequest::SetSlowmode {
            channel_id,
            seconds,
        } => handle_set_slowmode(state, community_id, sender_pseudonym, &channel_id, seconds),
        _ => unreachable!(),
    };

    // On any successful channel/category mutation, re-publish to DHT
    let should_publish = matches!(
        resp,
        CommunityResponse::ChannelCreated { .. }
            | CommunityResponse::CategoryCreated { .. }
            | CommunityResponse::Ok
    );
    if should_publish {
        let st = Arc::clone(state);
        let cid = community_id.to_string();
        tokio::spawn(async move {
            community_host::publish_channels(&st, &cid).await;
        });

        // Broadcast updated channel/category structure to all online members
        let (channels_dto, categories_dto) = {
            let hosted = state.hosted.read();
            if let Some(community) = hosted.get(community_id) {
                (channels_to_dto(&community.channels), categories_to_dto(&community.categories))
            } else {
                (Vec::new(), Vec::new())
            }
        };
        broadcast_to_members(
            state,
            community_id,
            sender_pseudonym,
            &CommunityBroadcast::ChannelsUpdated {
                community_id: community_id.to_string(),
                channels: channels_dto,
                categories: categories_dto,
            },
        );
    }

    resp
}

// ---------------------------------------------------------------------------
// Individual handlers
// ---------------------------------------------------------------------------

fn handle_create_channel(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    name: &str,
    channel_type: &str,
    category_id: Option<&str>,
) -> CommunityResponse {
    let mut hosted = state.hosted.write();

    let Some(community) = hosted.get_mut(community_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };
    if let Err(e) = verify_membership(community, sender_pseudonym) {
        return e;
    }

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS) {
        return e;
    }

    if !matches!(channel_type, "text" | "voice" | "announcement") {
        return CommunityResponse::Error {
            code: 400,
            message: format!(
                "invalid channel type '{channel_type}': must be 'text', 'voice', or 'announcement'"
            ),
        };
    }

    // Validate category_id if provided
    if let Some(cat_id) = category_id {
        if community.find_category(cat_id).is_none() {
            return CommunityResponse::Error {
                code: 404,
                message: format!("category '{cat_id}' not found"),
            };
        }
    }

    let channel_id = format!("channel_{}", hex::encode(super::rand_bytes(8)));
    let sort_order = i32::try_from(community.channels.len()).unwrap_or(i32::MAX);
    let channel = ServerChannel {
        id: channel_id.clone(),
        name: name.to_string(),
        channel_type: channel_type.to_string(),
        sort_order,
        permission_overwrites: Vec::new(),
        category_id: category_id.map(String::from),
        topic: String::new(),
        slowmode_seconds: 0,
    };

    {
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "INSERT INTO server_channels (community_id, id, name, channel_type, sort_order, category_id) VALUES (?,?,?,?,?,?)",
            params![community.community_id, channel.id, channel.name, channel.channel_type, channel.sort_order, channel.category_id],
        ) {
            tracing::error!(error = %e, "failed to insert channel into DB");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to create channel".into(),
            };
        }
    }

    community.channels.push(channel);
    audit::log_action(state, community_id, audit::AuditAction::CreateChannel, sender_pseudonym, None, Some(name));
    CommunityResponse::ChannelCreated { channel_id }
}

fn handle_delete_channel(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
) -> CommunityResponse {
    let mut hosted = state.hosted.write();

    let Some(community) = hosted.get_mut(community_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };
    if let Err(e) = verify_membership(community, sender_pseudonym) {
        return e;
    }

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS) {
        return e;
    }

    community.channels.retain(|ch| ch.id != channel_id);

    {
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "DELETE FROM server_channels WHERE community_id = ? AND id = ?",
            params![community.community_id, channel_id],
        ) {
            tracing::error!(error = %e, "failed to delete channel from DB");
        }
    }

    audit::log_action(state, community_id, audit::AuditAction::DeleteChannel, sender_pseudonym, None, Some(channel_id));
    CommunityResponse::Ok
}

fn handle_rename_channel(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    new_name: &str,
) -> CommunityResponse {
    let mut hosted = state.hosted.write();

    let Some(community) = hosted.get_mut(community_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };
    if let Err(e) = verify_membership(community, sender_pseudonym) {
        return e;
    }

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS) {
        return e;
    }

    let Some(channel) = community.find_channel_mut(channel_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "channel not found".into(),
        };
    };

    channel.name = new_name.to_string();

    {
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "UPDATE server_channels SET name = ? WHERE community_id = ? AND id = ?",
            params![new_name, community.community_id, channel_id],
        ) {
            tracing::error!(error = %e, "failed to rename channel in DB");
        }
    }

    audit::log_action(state, community_id, audit::AuditAction::RenameChannel, sender_pseudonym, None, Some(new_name));
    CommunityResponse::Ok
}

fn handle_create_category(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    name: &str,
) -> CommunityResponse {
    let mut hosted = state.hosted.write();
    let Some(community) = hosted.get_mut(community_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };
    if let Err(e) = verify_membership(community, sender_pseudonym) {
        return e;
    }
    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS) {
        return e;
    }

    let category_id = format!("cat_{}", hex::encode(super::rand_bytes(8)));
    let sort_order = i32::try_from(community.categories.len()).unwrap_or(i32::MAX);

    {
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "INSERT INTO server_categories (community_id, id, name, sort_order) VALUES (?,?,?,?)",
            params![community_id, category_id, name, sort_order],
        ) {
            tracing::error!(error = %e, "failed to insert category into DB");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to create category".into(),
            };
        }
    }

    community.categories.push(ServerCategory {
        id: category_id.clone(),
        name: name.to_string(),
        sort_order,
    });

    audit::log_action(state, community_id, audit::AuditAction::CreateCategory, sender_pseudonym, None, Some(name));
    CommunityResponse::CategoryCreated { category_id }
}

fn handle_delete_category(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    category_id: &str,
) -> CommunityResponse {
    let mut hosted = state.hosted.write();
    let Some(community) = hosted.get_mut(community_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };
    if let Err(e) = verify_membership(community, sender_pseudonym) {
        return e;
    }
    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS) {
        return e;
    }

    if community.find_category(category_id).is_none() {
        return CommunityResponse::Error {
            code: 404,
            message: "category not found".into(),
        };
    }

    // Remove category and unset category_id on channels that belonged to it
    community.categories.retain(|c| c.id != category_id);
    for ch in &mut community.channels {
        if ch.category_id.as_deref() == Some(category_id) {
            ch.category_id = None;
        }
    }

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "DELETE FROM server_categories WHERE community_id = ? AND id = ?",
            params![community_id, category_id],
        );
        let _ = db.execute(
            "UPDATE server_channels SET category_id = NULL WHERE community_id = ? AND category_id = ?",
            params![community_id, category_id],
        );
    }

    audit::log_action(state, community_id, audit::AuditAction::DeleteCategory, sender_pseudonym, None, Some(category_id));
    CommunityResponse::Ok
}

fn handle_rename_category(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    category_id: &str,
    new_name: &str,
) -> CommunityResponse {
    let mut hosted = state.hosted.write();
    let Some(community) = hosted.get_mut(community_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };
    if let Err(e) = verify_membership(community, sender_pseudonym) {
        return e;
    }
    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS) {
        return e;
    }

    let Some(cat) = community.categories.iter_mut().find(|c| c.id == category_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "category not found".into(),
        };
    };
    cat.name = new_name.to_string();

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "UPDATE server_categories SET name = ? WHERE community_id = ? AND id = ?",
            params![new_name, community_id, category_id],
        );
    }

    audit::log_action(state, community_id, audit::AuditAction::RenameCategory, sender_pseudonym, None, Some(new_name));
    CommunityResponse::Ok
}

fn handle_move_channel(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    category_id: Option<&str>,
) -> CommunityResponse {
    let mut hosted = state.hosted.write();
    let Some(community) = hosted.get_mut(community_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };
    if let Err(e) = verify_membership(community, sender_pseudonym) {
        return e;
    }
    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS) {
        return e;
    }

    // Validate target category exists (if provided)
    if let Some(cat_id) = category_id {
        if community.find_category(cat_id).is_none() {
            return CommunityResponse::Error {
                code: 404,
                message: format!("category '{cat_id}' not found"),
            };
        }
    }

    let Some(ch) = community.find_channel_mut(channel_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "channel not found".into(),
        };
    };
    ch.category_id = category_id.map(String::from);

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "UPDATE server_channels SET category_id = ? WHERE community_id = ? AND id = ?",
            params![category_id, community_id, channel_id],
        );
    }

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::MoveChannel,
        sender_pseudonym,
        Some(channel_id),
        category_id,
    );
    CommunityResponse::Ok
}

fn handle_reorder_categories(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    category_ids: &[String],
) -> CommunityResponse {
    let mut hosted = state.hosted.write();
    let Some(community) = hosted.get_mut(community_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };
    if let Err(e) = verify_membership(community, sender_pseudonym) {
        return e;
    }
    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS) {
        return e;
    }

    // Update sort_order based on position in the provided list
    for (i, cat_id) in category_ids.iter().enumerate() {
        let order = i32::try_from(i).unwrap_or(i32::MAX);
        if let Some(cat) = community.categories.iter_mut().find(|c| c.id == *cat_id) {
            cat.sort_order = order;
        }
    }
    community.categories.sort_by_key(|c| c.sort_order);

    {
        let db = crate::db_helpers::lock_db(&state.db);
        for (i, cat_id) in category_ids.iter().enumerate() {
            let order = i32::try_from(i).unwrap_or(i32::MAX);
            let _ = db.execute(
                "UPDATE server_categories SET sort_order = ? WHERE community_id = ? AND id = ?",
                params![order, community_id, cat_id],
            );
        }
    }

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::ReorderCategories,
        sender_pseudonym,
        None,
        None,
    );
    CommunityResponse::Ok
}

fn handle_set_channel_topic(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    topic: &str,
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
        if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS)
        {
            return e;
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);
    if let Err(e) = db.execute(
        "UPDATE server_channels SET topic = ? WHERE community_id = ? AND id = ?",
        params![topic, community_id, channel_id],
    ) {
        tracing::error!(error = %e, "failed to set channel topic");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to set topic".into(),
        };
    }
    drop(db);

    {
        let mut hosted = state.hosted.write();
        if let Some(community) = hosted.get_mut(community_id) {
            if let Some(ch) = community.find_channel_mut(channel_id) {
                ch.topic = topic.to_string();
            }
        }
    }

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::SetChannelTopic,
        sender_pseudonym,
        Some(channel_id),
        Some(topic),
    );
    CommunityResponse::Ok
}

fn handle_reorder_channels(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_ids: &[String],
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
        if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS)
        {
            return e;
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);
    for (i, cid) in channel_ids.iter().enumerate() {
        let order = i32::try_from(i).unwrap_or(i32::MAX);
        let _ = db.execute(
            "UPDATE server_channels SET sort_order = ? WHERE community_id = ? AND id = ?",
            params![order, community_id, cid],
        );
    }
    drop(db);

    {
        let mut hosted = state.hosted.write();
        if let Some(community) = hosted.get_mut(community_id) {
            for (i, cid) in channel_ids.iter().enumerate() {
                if let Some(ch) = community.find_channel_mut(cid) {
                    ch.sort_order = i32::try_from(i).unwrap_or(i32::MAX);
                }
            }
            community.channels.sort_by_key(|ch| ch.sort_order);
        }
    }

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::ReorderChannels,
        sender_pseudonym,
        None,
        None,
    );
    CommunityResponse::Ok
}

fn handle_set_slowmode(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    seconds: u32,
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
        if let Err(e) =
            check_permission(community, sender_pseudonym, permissions::MANAGE_CHANNELS)
        {
            return e;
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);
    if let Err(e) = db.execute(
        "UPDATE server_channels SET slowmode_seconds = ? WHERE community_id = ? AND id = ?",
        params![seconds, community_id, channel_id],
    ) {
        tracing::error!(error = %e, "failed to set slowmode");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to set slowmode".into(),
        };
    }
    drop(db);

    let mut hosted = state.hosted.write();
    if let Some(community) = hosted.get_mut(community_id) {
        if let Some(ch) = community.find_channel_mut(channel_id) {
            ch.slowmode_seconds = seconds;
        }
    }

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::SetSlowmode,
        sender_pseudonym,
        Some(channel_id),
        Some(&seconds.to_string()),
    );
    CommunityResponse::Ok
}

// ---------------------------------------------------------------------------
// DTO conversion helpers
// ---------------------------------------------------------------------------

fn channels_to_dto(channels: &[ServerChannel]) -> Vec<ChannelInfoDto> {
    channels
        .iter()
        .map(|ch| ChannelInfoDto {
            id: ch.id.clone(),
            name: ch.name.clone(),
            channel_type: ch.channel_type.clone(),
            category_id: ch.category_id.clone(),
            topic: ch.topic.clone(),
            slowmode_seconds: ch.slowmode_seconds,
        })
        .collect()
}

fn categories_to_dto(categories: &[ServerCategory]) -> Vec<CategoryDto> {
    categories
        .iter()
        .map(|cat| CategoryDto {
            id: cat.id.clone(),
            name: cat.name.clone(),
            sort_order: cat.sort_order,
        })
        .collect()
}
