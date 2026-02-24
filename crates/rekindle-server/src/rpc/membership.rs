use std::sync::Arc;

use rekindle_protocol::dht::community::{permissions, ROLE_EVERYONE_ID};
use rekindle_protocol::messaging::envelope::{
    CategoryDto, ChannelInfoDto, CommunityBroadcast, CommunityResponse, MemberInfoDto, RoleDto,
};
use rusqlite::params;

use crate::audit;
use crate::community_host;
use crate::mek;
use crate::server_state::{HostedCommunity, ServerMember, ServerState};

use super::broadcast::broadcast_to_members;
use super::invites::validate_and_consume_invite;
use super::permissions::{
    categories_to_dto, check_hierarchy, check_permission, find_community_for_route,
    roles_to_dto, verify_membership,
};

/// Result tuple returned by `add_new_member` on successful join.
type JoinResult = (Vec<u8>, u64, Vec<ChannelInfoDto>, Vec<CategoryDto>, Vec<u32>, Vec<RoleDto>);

/// Build a `Vec<MemberInfoDto>` from the community's current member list.
fn members_to_dto(community: &HostedCommunity) -> Vec<MemberInfoDto> {
    community
        .members
        .iter()
        .map(|m| MemberInfoDto {
            pseudonym_key: m.pseudonym_key_hex.clone(),
            display_name: m.display_name.clone(),
            role_ids: m.role_ids.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Join / Rejoin
// ---------------------------------------------------------------------------

fn build_rejoin_response(community: &HostedCommunity, pseudonym_pubkey: &str) -> CommunityResponse {
    let channels = community
        .channels
        .iter()
        .map(|ch| ChannelInfoDto {
            id: ch.id.clone(),
            name: ch.name.clone(),
            channel_type: ch.channel_type.clone(),
            category_id: ch.category_id.clone(),
            topic: ch.topic.clone(),
            slowmode_seconds: ch.slowmode_seconds,
        })
        .collect();

    let categories = categories_to_dto(community);

    let role_ids = community
        .find_member(pseudonym_pubkey)
        .map_or_else(Vec::new, |m| m.role_ids.clone());

    let mek_payload = community.mek.to_wire_bytes();

    let members = members_to_dto(community);

    CommunityResponse::Joined {
        mek_encrypted: mek_payload,
        mek_generation: community.mek.generation(),
        channels,
        categories,
        role_ids,
        roles: roles_to_dto(community),
        members,
    }
}

fn handle_rejoin(
    state: &Arc<ServerState>,
    community_id: &str,
    pseudonym_pubkey: &str,
    member_route_blob: Option<&[u8]>,
) -> Option<CommunityResponse> {
    let mut hosted = state.hosted.write();
    let community = hosted.get_mut(community_id)?;
    let member = community.find_member_mut(pseudonym_pubkey)?;

    if let Some(blob) = member_route_blob {
        member.route_blob = Some(blob.to_vec());
        crate::db_helpers::db_fire(&state.db, "update member route blob", |db| {
            db.execute(
                "UPDATE server_members SET route_blob = ? WHERE community_id = ? AND pseudonym_key_hex = ?",
                params![blob, community_id, pseudonym_pubkey],
            )?;
            Ok(())
        });
    }
    Some(build_rejoin_response(community, pseudonym_pubkey))
}

fn add_new_member(
    state: &Arc<ServerState>,
    community_id: &str,
    pseudonym_pubkey: &str,
    display_name: &str,
    member_route_blob: Option<&[u8]>,
) -> Option<JoinResult> {
    let now = rekindle_utils::timestamp_secs_i64();

    // Determine role IDs — first member is the creator (gets Owner + Admin + Mod + Member + @everyone)
    let is_first_member = {
        let hosted = state.hosted.read();
        hosted
            .get(community_id)
            .is_some_and(|c| c.members.is_empty())
    };
    let default_role_ids = if is_first_member {
        vec![ROLE_EVERYONE_ID, 1, 2, 3, 4] // @everyone, Member, Moderator, Admin, Owner
    } else {
        vec![ROLE_EVERYONE_ID, 1] // @everyone + Member
    };

    // Insert member into DB
    {
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "INSERT OR IGNORE INTO server_members (community_id, pseudonym_key_hex, display_name, joined_at) VALUES (?,?,?,?)",
            params![community_id, pseudonym_pubkey, display_name, now],
        ) {
            tracing::error!(error = %e, "failed to insert member into DB");
        }
        // Insert role assignments
        for role_id in &default_role_ids {
            let _ = db.execute(
                "INSERT OR IGNORE INTO server_member_roles (community_id, pseudonym_key_hex, role_id) VALUES (?,?,?)",
                params![community_id, pseudonym_pubkey, role_id],
            );
        }
    }

    let mut hosted = state.hosted.write();
    let community = hosted.get_mut(community_id)?;

    // If first member, set them as the community creator
    if is_first_member {
        community.creator_pseudonym_hex = pseudonym_pubkey.to_string();
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "UPDATE hosted_communities SET creator_pseudonym = ? WHERE id = ?",
            params![pseudonym_pubkey, community_id],
        );
    }

    community.members.push(ServerMember {
        pseudonym_key_hex: pseudonym_pubkey.to_string(),
        display_name: display_name.to_string(),
        role_ids: default_role_ids.clone(),
        joined_at: now,
        route_blob: member_route_blob.map(<[u8]>::to_vec),
        timeout_until: None,
        online_status: "online".into(),
    });

    if let Some(blob) = member_route_blob {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "UPDATE server_members SET route_blob = ? WHERE community_id = ? AND pseudonym_key_hex = ?",
            params![blob, community.community_id, pseudonym_pubkey],
        );
    }

    let channels = community
        .channels
        .iter()
        .map(|ch| ChannelInfoDto {
            id: ch.id.clone(),
            name: ch.name.clone(),
            channel_type: ch.channel_type.clone(),
            category_id: ch.category_id.clone(),
            topic: ch.topic.clone(),
            slowmode_seconds: ch.slowmode_seconds,
        })
        .collect();

    let categories = categories_to_dto(community);
    let roles_dto = roles_to_dto(community);

    let mek_generation = community.mek.generation();
    let mek_payload = community.mek.to_wire_bytes();

    Some((
        mek_payload,
        mek_generation,
        channels,
        categories,
        default_role_ids,
        roles_dto,
    ))
}

pub(super) async fn handle_join(
    state: &Arc<ServerState>,
    pseudonym_pubkey: &str,
    display_name: &str,
    invite_code: Option<&str>,
    member_route_blob: Option<Vec<u8>>,
    incoming_route_id: Option<&veilid_core::RouteId>,
    ipc_community_id: Option<&str>,
) -> CommunityResponse {
    // IPC path: community_id is provided directly. Veilid path: look up by route.
    let community_id = ipc_community_id
        .map(String::from)
        .or_else(|| incoming_route_id.and_then(|rid| find_community_for_route(state, rid)));
    let Some(community_id) = community_id else {
        return CommunityResponse::Error {
            code: 404,
            message: "no hosted community matches this route".into(),
        };
    };

    // Verify the community actually exists when coming via IPC
    if ipc_community_id.is_some() {
        let hosted = state.hosted.read();
        if !hosted.contains_key(&community_id) {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        }
    }

    // Check if banned
    {
        let db = crate::db_helpers::lock_db(&state.db);
        let is_banned: bool = db
            .query_row(
                "SELECT 1 FROM banned_members WHERE community_id = ? AND pseudonym_key_hex = ?",
                params![community_id, pseudonym_pubkey],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if is_banned {
            return CommunityResponse::Error {
                code: 403,
                message: "you are banned from this community".into(),
            };
        }
    }

    // Validate invite code if provided (for new members only — rejoins skip this)
    if let Some(code) = invite_code {
        if let Err(e) = validate_and_consume_invite(state, &community_id, code) {
            return e;
        }
    }

    if let Some(resp) = handle_rejoin(
        state,
        &community_id,
        pseudonym_pubkey,
        member_route_blob.as_deref(),
    ) {
        return resp;
    }

    let Some((mek_payload, mek_generation, channels, categories, role_ids, roles)) =
        add_new_member(
            state,
            &community_id,
            pseudonym_pubkey,
            display_name,
            member_route_blob.as_deref(),
        )
    else {
        return CommunityResponse::Error {
            code: 404,
            message: "community not found".into(),
        };
    };

    community_host::publish_member_roster(state, &community_id).await;

    broadcast_to_members(
        state,
        &community_id,
        pseudonym_pubkey,
        &CommunityBroadcast::MemberJoined {
            community_id: community_id.clone(),
            pseudonym_key: pseudonym_pubkey.to_string(),
            display_name: display_name.to_string(),
            role_ids: role_ids.clone(),
        },
    );

    // Build member list after adding the new member (includes the joiner)
    let members = {
        let hosted = state.hosted.read();
        hosted
            .get(&community_id)
            .map(members_to_dto)
            .unwrap_or_default()
    };

    CommunityResponse::Joined {
        mek_encrypted: mek_payload,
        mek_generation,
        channels,
        categories,
        role_ids,
        roles,
        members,
    }
}

// ---------------------------------------------------------------------------
// MEK request
// ---------------------------------------------------------------------------

pub(super) fn handle_request_mek(
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

    let mek_payload = community.mek.to_wire_bytes();

    CommunityResponse::Mek {
        mek_encrypted: mek_payload,
        mek_generation: community.mek.generation(),
    }
}

// ---------------------------------------------------------------------------
// Leave / Kick
// ---------------------------------------------------------------------------

pub(super) async fn handle_leave(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        if let Some(community) = hosted.get(community_id) {
            if let Err(e) = verify_membership(community, sender_pseudonym) {
                return e;
            }
        }
    }

    {
        let db = crate::db_helpers::lock_db(&state.db);
        // CASCADE will also delete from server_member_roles and server_member_timeouts
        if let Err(e) = db.execute(
            "DELETE FROM server_members WHERE community_id = ? AND pseudonym_key_hex = ?",
            params![community_id, sender_pseudonym],
        ) {
            tracing::error!(error = %e, "failed to delete member from DB");
        }
    }

    {
        let mut hosted = state.hosted.write();
        if let Some(community) = hosted.get_mut(community_id) {
            community
                .members
                .retain(|m| m.pseudonym_key_hex != sender_pseudonym);
            let new_gen = community.mek.generation() + 1;
            community.mek = mek::rotate_mek(state, community_id, new_gen);
        }
    }

    community_host::publish_member_roster(state, community_id).await;
    community_host::publish_mek_bundle(state, community_id).await;

    broadcast_to_members(
        state,
        community_id,
        sender_pseudonym,
        &CommunityBroadcast::MemberRemoved {
            community_id: community_id.to_string(),
            pseudonym_key: sender_pseudonym.to_string(),
        },
    );

    let new_gen = {
        let hosted = state.hosted.read();
        hosted.get(community_id).map(|c| c.mek.generation())
    };
    if let Some(gen) = new_gen {
        broadcast_to_members(
            state,
            community_id,
            sender_pseudonym,
            &CommunityBroadcast::MEKRotated {
                community_id: community_id.to_string(),
                new_generation: gen,
            },
        );
    }

    tracing::info!(community = %community_id, member = %sender_pseudonym, "member left community");
    CommunityResponse::Ok
}

pub(super) async fn handle_kick(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 403,
                message: "not a member".into(),
            };
        };

        if let Err(e) = check_permission(community, sender_pseudonym, permissions::KICK_MEMBERS) {
            return e;
        }

        if sender_pseudonym == target_pseudonym {
            return CommunityResponse::Error {
                code: 400,
                message: "cannot kick yourself — use Leave instead".into(),
            };
        }

        if let Err(e) = check_hierarchy(community, sender_pseudonym, target_pseudonym) {
            return e;
        }
    }

    let resp = handle_leave(state, community_id, target_pseudonym).await;
    if matches!(resp, CommunityResponse::Ok) {
        audit::log_action(state, community_id, audit::AuditAction::Kick, sender_pseudonym, Some(target_pseudonym), None);
    }
    resp
}

// ---------------------------------------------------------------------------
// MEK rotation
// ---------------------------------------------------------------------------

pub(super) async fn handle_rotate_mek(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
) -> CommunityResponse {
    let new_gen = {
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

        if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_COMMUNITY)
        {
            return e;
        }

        let new_gen = community.mek.generation() + 1;
        community.mek = mek::rotate_mek(state, community_id, new_gen);
        new_gen
    };

    community_host::publish_mek_bundle(state, community_id).await;

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::MEKRotated {
            community_id: community_id.to_string(),
            new_generation: new_gen,
        },
    );

    audit::log_action(state, community_id, audit::AuditAction::RotateMek, sender_pseudonym, None, None);
    CommunityResponse::Ok
}

// ---------------------------------------------------------------------------
// Community metadata update
// ---------------------------------------------------------------------------

pub(super) async fn handle_update_community(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    new_name: Option<&str>,
    new_description: Option<&str>,
) -> CommunityResponse {
    {
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

        if let Err(e) = check_permission(community, sender_pseudonym, permissions::MANAGE_COMMUNITY)
        {
            return e;
        }

        if let Some(n) = new_name {
            community.name = n.to_string();
        }
        if let Some(d) = new_description {
            community.description = d.to_string();
        }

        {
            let db = crate::db_helpers::lock_db(&state.db);
            if let Some(n) = new_name {
                let _ = db.execute(
                    "UPDATE hosted_communities SET name = ? WHERE id = ?",
                    params![n, community_id],
                );
            }
            if let Some(d) = new_description {
                let _ = db.execute(
                    "UPDATE hosted_communities SET description = ? WHERE id = ?",
                    params![d, community_id],
                );
            }
        }
    }

    let name = {
        let hosted = state.hosted.read();
        hosted.get(community_id).map(|c| c.name.clone())
    };
    if let Some(name) = name {
        community_host::publish_metadata(state, community_id, &name).await;
    }

    audit::log_action(state, community_id, audit::AuditAction::UpdateCommunity, sender_pseudonym, None, None);
    CommunityResponse::CommunityUpdated
}

// ---------------------------------------------------------------------------
// Ban / Unban
// ---------------------------------------------------------------------------

pub(super) async fn handle_ban(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
) -> CommunityResponse {
    let display_name = {
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

        if let Err(e) = check_permission(community, sender_pseudonym, permissions::BAN_MEMBERS) {
            return e;
        }

        if sender_pseudonym == target_pseudonym {
            return CommunityResponse::Error {
                code: 400,
                message: "cannot ban yourself".into(),
            };
        }

        if let Err(e) = check_hierarchy(community, sender_pseudonym, target_pseudonym) {
            return e;
        }

        community
            .find_member(target_pseudonym)
            .map_or_else(String::new, |m| m.display_name.clone())
    };

    {
        let now = rekindle_utils::timestamp_secs_i64();
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "INSERT OR REPLACE INTO banned_members (community_id, pseudonym_key_hex, display_name, banned_at) VALUES (?,?,?,?)",
            params![community_id, target_pseudonym, display_name, now],
        ) {
            tracing::error!(error = %e, "failed to insert ban record");
        }
    }

    audit::log_action(state, community_id, audit::AuditAction::Ban, sender_pseudonym, Some(target_pseudonym), None);
    handle_leave(state, community_id, target_pseudonym).await
}

pub(super) fn handle_unban(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::BAN_MEMBERS) {
        return e;
    }

    {
        let db = crate::db_helpers::lock_db(&state.db);
        if let Err(e) = db.execute(
            "DELETE FROM banned_members WHERE community_id = ? AND pseudonym_key_hex = ?",
            params![community.community_id, target_pseudonym],
        ) {
            tracing::error!(error = %e, "failed to remove ban record");
        }
    }

    tracing::info!(community = %community.community_id, member = %target_pseudonym, "member unbanned");
    audit::log_action(state, community_id, audit::AuditAction::Unban, sender_pseudonym, Some(target_pseudonym), None);
    CommunityResponse::Ok
}

pub(super) fn handle_get_ban_list(
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::BAN_MEMBERS) {
        return e;
    }

    let db = crate::db_helpers::lock_db(&state.db);

    let banned = db
        .prepare(
            "SELECT pseudonym_key_hex, display_name, banned_at FROM banned_members WHERE community_id = ? ORDER BY banned_at DESC",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![community.community_id], |row| {
                let banned_at: i64 = row.get(2)?;
                Ok(rekindle_protocol::messaging::envelope::BannedMemberDto {
                    pseudonym_key: row.get(0)?,
                    display_name: row.get(1)?,
                    banned_at: banned_at.try_into().unwrap_or(0u64),
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();

    CommunityResponse::BanList { banned }
}

// ---------------------------------------------------------------------------
// Timeout / Remove timeout
// ---------------------------------------------------------------------------

pub(super) fn handle_timeout_member(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
    duration_seconds: u64,
    reason: Option<&String>,
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MODERATE_MEMBERS) {
        return e;
    }

    if let Err(e) = check_hierarchy(community, sender_pseudonym, target_pseudonym) {
        return e;
    }

    let now = rekindle_utils::timestamp_secs();
    let timeout_until = now + duration_seconds;

    let Some(member) = community.find_member_mut(target_pseudonym) else {
        return CommunityResponse::Error {
            code: 404,
            message: "target member not found".into(),
        };
    };

    member.timeout_until = Some(timeout_until);

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "INSERT OR REPLACE INTO server_member_timeouts (community_id, pseudonym_key_hex, timeout_until, reason) VALUES (?,?,?,?)",
            params![community.community_id, target_pseudonym, timeout_until.cast_signed(), reason],
        );
    }

    drop(hosted); // Release write lock before broadcasting

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::MemberTimedOut {
            community_id: community_id.to_string(),
            pseudonym_key: target_pseudonym.to_string(),
            timeout_until: Some(timeout_until),
        },
    );

    audit::log_action(state, community_id, audit::AuditAction::TimeoutMember, sender_pseudonym, Some(target_pseudonym), reason.map(String::as_str));
    CommunityResponse::Ok
}

pub(super) fn handle_remove_timeout(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
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

    if let Err(e) = check_permission(community, sender_pseudonym, permissions::MODERATE_MEMBERS) {
        return e;
    }

    if let Some(member) = community.find_member_mut(target_pseudonym) {
        member.timeout_until = None;
    }

    {
        let db = crate::db_helpers::lock_db(&state.db);
        let _ = db.execute(
            "DELETE FROM server_member_timeouts WHERE community_id = ? AND pseudonym_key_hex = ?",
            params![community.community_id, target_pseudonym],
        );
    }

    drop(hosted); // Release write lock before broadcasting

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::MemberTimedOut {
            community_id: community_id.to_string(),
            pseudonym_key: target_pseudonym.to_string(),
            timeout_until: None,
        },
    );

    audit::log_action(state, community_id, audit::AuditAction::RemoveTimeout, sender_pseudonym, Some(target_pseudonym), None);
    CommunityResponse::Ok
}
