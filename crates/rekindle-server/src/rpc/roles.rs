use std::sync::Arc;

use rekindle_protocol::dht::community::{
    permissions, OverwriteType, PermissionOverwrite, RoleDefinition, ROLE_EVERYONE_ID,
};
use rekindle_protocol::messaging::envelope::{CommunityBroadcast, CommunityResponse};
use rusqlite::params;

use crate::audit;
use crate::server_state::ServerState;

use super::broadcast::broadcast_to_members;
use super::permissions::{
    check_hierarchy, check_permission, highest_role_position, member_base_permissions,
    roles_to_dto, verify_membership,
};

pub(super) fn handle_create_role(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    name: &str,
    color: u32,
    perms: u64,
    hoist: bool,
    mentionable: bool,
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_ROLES) {
        return e;
    }

    // Generate next role ID
    let next_id = community.roles.iter().map(|r| r.id).max().unwrap_or(0) + 1;
    // Position: one below sender's highest (new roles start below the creator)
    let sender_pos = highest_role_position(community, sender_pseudonym);
    let position = sender_pos.saturating_sub(1).max(1); // at least 1, never 0 (@everyone)

    let role = RoleDefinition {
        id: next_id,
        name: name.to_string(),
        color,
        permissions: perms,
        position,
        hoist,
        mentionable,
    };

    {
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "INSERT INTO server_roles (community_id, id, name, color, permissions, position, hoist, mentionable) VALUES (?,?,?,?,?,?,?,?)",
            params![
                community.community_id,
                role.id,
                role.name,
                role.color,
                role.permissions.cast_signed(),
                role.position,
                i32::from(role.hoist),
                i32::from(role.mentionable),
            ],
        ) {
            tracing::error!(error = %e, "failed to insert role into DB");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to create role".into(),
            };
        }
    }

    community.roles.push(role);
    audit::log_action(
        state,
        community_id,
        audit::AuditAction::CreateRole,
        sender_pseudonym,
        None,
        Some(name),
    );
    CommunityResponse::RoleCreated { role_id: next_id }
}

pub(super) fn handle_edit_role(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    role_id: u32,
    name: Option<&String>,
    color: Option<u32>,
    perms: Option<u64>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_ROLES) {
        return e;
    }

    let sender_pos = highest_role_position(community, sender_pseudonym);
    let sender_is_admin =
        permissions::is_administrator(member_base_permissions(community, sender_pseudonym));

    // Check role exists and position constraint before mutating
    let target_pos = community
        .roles
        .iter()
        .find(|r| r.id == role_id)
        .map(|r| r.position);
    let Some(target_position) = target_pos else {
        return CommunityResponse::Error {
            code: 404,
            message: "role not found".into(),
        };
    };

    // Can only edit roles below own position (unless admin)
    if target_position >= sender_pos && !sender_is_admin {
        return CommunityResponse::Error {
            code: 403,
            message: "cannot edit a role at or above your position".into(),
        };
    }

    let Some(role) = community.roles.iter_mut().find(|r| r.id == role_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "role not found".into(),
        };
    };

    if let Some(n) = &name {
        role.name.clone_from(n);
    }
    if let Some(c) = color {
        role.color = c;
    }
    if let Some(p) = perms {
        role.permissions = p;
    }
    if let Some(pos) = position {
        role.position = pos;
    }
    if let Some(h) = hoist {
        role.hoist = h;
    }
    if let Some(m) = mentionable {
        role.mentionable = m;
    }

    // Persist
    {
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "UPDATE server_roles SET name=?, color=?, permissions=?, position=?, hoist=?, mentionable=? WHERE community_id=? AND id=?",
            params![
                role.name,
                role.color,
                role.permissions.cast_signed(),
                role.position,
                i32::from(role.hoist),
                i32::from(role.mentionable),
                community.community_id,
                role_id,
            ],
        ) {
            tracing::error!(error = %e, "failed to update role in DB");
        }
    }

    let role_id_str = role_id.to_string();
    audit::log_action(
        state,
        community_id,
        audit::AuditAction::EditRole,
        sender_pseudonym,
        None,
        Some(&role_id_str),
    );
    CommunityResponse::Ok
}

pub(super) fn handle_delete_role(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    role_id: u32,
) -> CommunityResponse {
    if role_id == ROLE_EVERYONE_ID {
        return CommunityResponse::Error {
            code: 400,
            message: "cannot delete the @everyone role".into(),
        };
    }

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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_ROLES) {
        return e;
    }

    let sender_pos = highest_role_position(community, sender_pseudonym);
    let role_pos = community
        .roles
        .iter()
        .find(|r| r.id == role_id)
        .map_or(i32::MAX, |r| r.position);
    if role_pos >= sender_pos {
        return CommunityResponse::Error {
            code: 403,
            message: "cannot delete a role at or above your position".into(),
        };
    }

    community.roles.retain(|r| r.id != role_id);
    // Remove from all members
    for member in &mut community.members {
        member.role_ids.retain(|rid| *rid != role_id);
    }

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "DELETE FROM server_roles WHERE community_id = ? AND id = ?",
            params![community.community_id, role_id],
        );
        let _ = db.execute(
            "DELETE FROM server_member_roles WHERE community_id = ? AND role_id = ?",
            params![community.community_id, role_id],
        );
    }

    let role_id_str = role_id.to_string();
    audit::log_action(
        state,
        community_id,
        audit::AuditAction::DeleteRole,
        sender_pseudonym,
        None,
        Some(&role_id_str),
    );
    CommunityResponse::Ok
}

pub(super) fn handle_assign_role(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
    role_id: u32,
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_ROLES) {
        return e;
    }

    if let Err(e) = check_hierarchy(community, sender_pseudonym, target_pseudonym) {
        return e;
    }

    // Can only assign roles below own position
    let sender_pos = highest_role_position(community, sender_pseudonym);
    let role_pos = community
        .roles
        .iter()
        .find(|r| r.id == role_id)
        .map_or(i32::MAX, |r| r.position);
    if role_pos >= sender_pos {
        return CommunityResponse::Error {
            code: 403,
            message: "cannot assign a role at or above your position".into(),
        };
    }

    let Some(member) = community.find_member_mut(target_pseudonym) else {
        return CommunityResponse::Error {
            code: 404,
            message: "target member not found".into(),
        };
    };

    if !member.role_ids.contains(&role_id) {
        member.role_ids.push(role_id);
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "INSERT OR IGNORE INTO server_member_roles (community_id, pseudonym_key_hex, role_id) VALUES (?,?,?)",
            params![community.community_id, target_pseudonym, role_id],
        );
    }

    let role_id_str = role_id.to_string();
    audit::log_action(
        state,
        community_id,
        audit::AuditAction::AssignRole,
        sender_pseudonym,
        Some(target_pseudonym),
        Some(&role_id_str),
    );
    CommunityResponse::Ok
}

pub(super) fn handle_unassign_role(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
    role_id: u32,
) -> CommunityResponse {
    if role_id == ROLE_EVERYONE_ID {
        return CommunityResponse::Error {
            code: 400,
            message: "cannot remove the @everyone role".into(),
        };
    }

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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_ROLES) {
        return e;
    }

    if let Err(e) = check_hierarchy(community, sender_pseudonym, target_pseudonym) {
        return e;
    }

    let Some(member) = community.find_member_mut(target_pseudonym) else {
        return CommunityResponse::Error {
            code: 404,
            message: "target member not found".into(),
        };
    };

    member.role_ids.retain(|rid| *rid != role_id);

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "DELETE FROM server_member_roles WHERE community_id = ? AND pseudonym_key_hex = ? AND role_id = ?",
            params![community.community_id, target_pseudonym, role_id],
        );
    }

    let role_id_str = role_id.to_string();
    audit::log_action(
        state,
        community_id,
        audit::AuditAction::UnassignRole,
        sender_pseudonym,
        Some(target_pseudonym),
        Some(&role_id_str),
    );
    CommunityResponse::Ok
}

pub(super) fn handle_get_roles(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
) -> CommunityResponse {
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

    CommunityResponse::RolesList {
        roles: roles_to_dto(community),
    }
}

// ---------------------------------------------------------------------------
// Channel permission overwrites
// ---------------------------------------------------------------------------

pub(super) fn handle_set_channel_overwrite(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    target_type: &str,
    target_id: &str,
    allow: u64,
    deny: u64,
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_ROLES) {
        return e;
    }

    let ow_type = match target_type {
        "member" => OverwriteType::Member,
        _ => OverwriteType::Role,
    };

    let Some(channel) = community.find_channel_mut(channel_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "channel not found".into(),
        };
    };

    // Upsert
    if let Some(existing) = channel
        .permission_overwrites
        .iter_mut()
        .find(|o| o.target_type == ow_type && o.target_id == target_id)
    {
        existing.allow = allow;
        existing.deny = deny;
    } else {
        channel.permission_overwrites.push(PermissionOverwrite {
            target_type: ow_type,
            target_id: target_id.to_string(),
            allow,
            deny,
        });
    }

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "INSERT OR REPLACE INTO server_channel_overwrites (community_id, channel_id, target_type, target_id, allow_bits, deny_bits) VALUES (?,?,?,?,?,?)",
            params![community.community_id, channel_id, target_type, target_id, allow.cast_signed(), deny.cast_signed()],
        );
    }

    drop(hosted); // Release write lock before broadcasting

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::ChannelOverwriteChanged {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
        },
    );

    CommunityResponse::Ok
}

pub(super) fn handle_delete_channel_overwrite(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
    target_type: &str,
    target_id: &str,
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_ROLES) {
        return e;
    }

    let ow_type = match target_type {
        "member" => OverwriteType::Member,
        _ => OverwriteType::Role,
    };

    if let Some(channel) = community.find_channel_mut(channel_id) {
        channel
            .permission_overwrites
            .retain(|o| !(o.target_type == ow_type && o.target_id == target_id));
    }

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "DELETE FROM server_channel_overwrites WHERE community_id = ? AND channel_id = ? AND target_type = ? AND target_id = ?",
            params![community.community_id, channel_id, target_type, target_id],
        );
    }

    CommunityResponse::Ok
}
