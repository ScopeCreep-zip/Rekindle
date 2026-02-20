use std::sync::Arc;

use rekindle_protocol::dht::community::{
    permissions, OverwriteType, PermissionOverwrite, RoleDefinition, ROLE_EVERYONE_ID,
};
use rekindle_protocol::messaging::envelope::{
    ChannelInfoDto, ChannelMessageDto, CommunityBroadcast, CommunityRequest, CommunityResponse,
    RoleDto,
};
use rekindle_protocol::messaging::receiver::process_incoming;
use rusqlite::params;

use crate::community_host;
use crate::mek;
use crate::server_state::{HostedCommunity, ServerChannel, ServerMember, ServerState};

/// Result tuple returned by `add_new_member` on successful join.
type JoinResult = (Vec<u8>, u64, Vec<ChannelInfoDto>, Vec<u32>, Vec<RoleDto>);

// ---------------------------------------------------------------------------
// Permission helpers
// ---------------------------------------------------------------------------

/// Calculate a member's effective base permissions from their roles.
fn member_base_permissions(community: &HostedCommunity, pseudonym: &str) -> u64 {
    let member = community
        .members
        .iter()
        .find(|m| m.pseudonym_key_hex == pseudonym);
    let Some(member) = member else {
        return 0;
    };
    permissions::calculate_permissions(
        &member.role_ids,
        &community.roles,
        &[], // no channel overwrites for base perms
        pseudonym,
        member.timeout_until,
    )
}

/// Check that a sender has a required permission. Returns Err(CommunityResponse) on failure.
/// The community creator always passes (inherent owner bypass, like Discord's server owner).
fn check_permission(
    community: &HostedCommunity,
    sender_pseudonym: &str,
    required: u64,
) -> Result<(), CommunityResponse> {
    // Community creator always has all permissions
    if !community.creator_pseudonym_hex.is_empty()
        && community.creator_pseudonym_hex == sender_pseudonym
    {
        return Ok(());
    }
    let perms = member_base_permissions(community, sender_pseudonym);
    if permissions::has_permission(perms, required) {
        Ok(())
    } else {
        Err(CommunityResponse::Error {
            code: 403,
            message: "insufficient permissions".into(),
        })
    }
}

/// Get the highest role position for a member. Higher = more authority.
fn highest_role_position(community: &HostedCommunity, pseudonym: &str) -> i32 {
    let member = community
        .members
        .iter()
        .find(|m| m.pseudonym_key_hex == pseudonym);
    let Some(member) = member else {
        return -1;
    };
    member
        .role_ids
        .iter()
        .filter_map(|rid| community.roles.iter().find(|r| r.id == *rid))
        .map(|r| r.position)
        .max()
        .unwrap_or(-1)
}

/// Check that sender outranks target in role hierarchy.
/// The community creator always outranks everyone.
fn check_hierarchy(
    community: &HostedCommunity,
    sender_pseudonym: &str,
    target_pseudonym: &str,
) -> Result<(), CommunityResponse> {
    // Community creator always outranks everyone
    if !community.creator_pseudonym_hex.is_empty()
        && community.creator_pseudonym_hex == sender_pseudonym
    {
        return Ok(());
    }
    let sender_pos = highest_role_position(community, sender_pseudonym);
    let target_pos = highest_role_position(community, target_pseudonym);
    if sender_pos > target_pos {
        Ok(())
    } else {
        Err(CommunityResponse::Error {
            code: 403,
            message: "target has equal or higher role position".into(),
        })
    }
}

/// Build `RoleDto` vec from community roles.
fn roles_to_dto(community: &HostedCommunity) -> Vec<RoleDto> {
    community
        .roles
        .iter()
        .map(|r| RoleDto {
            id: r.id,
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position,
            hoist: r.hoist,
            mentionable: r.mentionable,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Routing helpers
// ---------------------------------------------------------------------------

/// Find which hosted community a member belongs to by their pseudonym key.
fn find_community_for_member(state: &Arc<ServerState>, pseudonym: &str) -> Option<String> {
    let hosted = state.hosted.read();
    hosted
        .values()
        .find(|c| c.members.iter().any(|m| m.pseudonym_key_hex == pseudonym))
        .map(|c| c.community_id.clone())
}

/// Find which hosted community a route belongs to by its private route ID.
fn find_community_for_route(
    state: &Arc<ServerState>,
    route_id: &veilid_core::RouteId,
) -> Option<String> {
    let hosted = state.hosted.read();
    hosted
        .values()
        .find(|c| c.route_id.as_ref() == Some(route_id))
        .map(|c| c.community_id.clone())
}

/// Resolve the target community ID for a non-Join request.
///
/// Priority: IPC-provided `community_id` > route-based lookup > member scan.
fn resolve_community_id(
    state: &Arc<ServerState>,
    sender_pseudonym: &str,
    incoming_route_id: Option<&veilid_core::RouteId>,
    ipc_community_id: Option<&str>,
) -> Option<String> {
    if let Some(cid) = ipc_community_id {
        return Some(cid.to_string());
    }
    if let Some(cid) = incoming_route_id.and_then(|r| find_community_for_route(state, r)) {
        return Some(cid);
    }
    find_community_for_member(state, sender_pseudonym)
}

/// Check that `sender_pseudonym` is a member of the community. Returns the
/// "not a member" error response on failure.
fn verify_membership(
    community: &HostedCommunity,
    sender_pseudonym: &str,
) -> Result<(), CommunityResponse> {
    if community.creator_pseudonym_hex == sender_pseudonym {
        return Ok(());
    }
    if community
        .members
        .iter()
        .any(|m| m.pseudonym_key_hex == sender_pseudonym)
    {
        return Ok(());
    }
    Err(CommunityResponse::Error {
        code: 403,
        message: "not a member".into(),
    })
}

// ---------------------------------------------------------------------------
// Main RPC dispatch
// ---------------------------------------------------------------------------

/// Handle a community RPC request arriving via local IPC (Unix socket).
///
/// Bypasses envelope parsing and signature verification — the Unix socket
/// is same-user-only, so authentication is implicit. The caller provides
/// the sender's pseudonym key directly.
///
/// Returns serialized `CommunityResponse` bytes (JSON).
pub async fn handle_community_rpc_direct(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym_key: &str,
    request_json: &str,
) -> Vec<u8> {
    let request: CommunityRequest = match serde_json::from_str(request_json) {
        Ok(r) => r,
        Err(e) => {
            return serde_json::to_vec(&CommunityResponse::Error {
                code: 400,
                message: format!("invalid request JSON: {e}"),
            })
            .unwrap_or_default();
        }
    };

    let response = dispatch_request(
        state,
        sender_pseudonym_key,
        request,
        None,
        Some(community_id),
    )
    .await;
    serde_json::to_vec(&response).unwrap_or_else(|_| {
        serde_json::to_vec(&CommunityResponse::Error {
            code: 500,
            message: "serialization failed".into(),
        })
        .unwrap_or_default()
    })
}

/// Handle an incoming community RPC request (from `app_call`).
///
/// Returns serialized `CommunityResponse` bytes for the reply.
pub async fn handle_community_request(
    state: &Arc<ServerState>,
    raw: &[u8],
    incoming_route_id: Option<&veilid_core::RouteId>,
) -> Vec<u8> {
    let response = process_request(state, raw, incoming_route_id).await;
    serde_json::to_vec(&response).unwrap_or_else(|_| {
        serde_json::to_vec(&CommunityResponse::Error {
            code: 500,
            message: "serialization failed".into(),
        })
        .unwrap_or_default()
    })
}

async fn process_request(
    state: &Arc<ServerState>,
    raw: &[u8],
    incoming_route_id: Option<&veilid_core::RouteId>,
) -> CommunityResponse {
    let envelope = match process_incoming(raw) {
        Ok(env) => env,
        Err(e) => {
            return CommunityResponse::Error {
                code: 400,
                message: format!("invalid envelope: {e}"),
            };
        }
    };

    let sender_pseudonym = hex::encode(&envelope.sender_key);

    let request: CommunityRequest = match serde_json::from_slice(&envelope.payload) {
        Ok(r) => r,
        Err(e) => {
            return CommunityResponse::Error {
                code: 400,
                message: format!("invalid request payload: {e}"),
            };
        }
    };

    dispatch_request(state, &sender_pseudonym, request, incoming_route_id, None).await
}

#[allow(clippy::too_many_lines)]
async fn dispatch_request(
    state: &Arc<ServerState>,
    sender_pseudonym: &str,
    request: CommunityRequest,
    incoming_route_id: Option<&veilid_core::RouteId>,
    ipc_community_id: Option<&str>,
) -> CommunityResponse {
    // Handle Join separately — the sender isn't a member yet, so community_id
    // resolution works differently (via IPC hint or route lookup).
    if let CommunityRequest::Join {
        pseudonym_pubkey,
        display_name,
        route_blob,
        ..
    } = request
    {
        if pseudonym_pubkey != sender_pseudonym {
            return CommunityResponse::Error {
                code: 403,
                message: "pseudonym mismatch: claimed key does not match envelope signature".into(),
            };
        }
        return handle_join(
            state,
            sender_pseudonym,
            &display_name,
            route_blob,
            incoming_route_id,
            ipc_community_id,
        )
        .await;
    }

    // For all non-Join requests, resolve the target community upfront.
    let Some(community_id) =
        resolve_community_id(state, sender_pseudonym, incoming_route_id, ipc_community_id)
    else {
        return CommunityResponse::Error {
            code: 403,
            message: "not a member of any hosted community".into(),
        };
    };

    match request {
        CommunityRequest::Join { .. } => unreachable!(),

        CommunityRequest::SendMessage {
            channel_id,
            ciphertext,
            mek_generation,
        } => handle_send_message(
            state,
            &community_id,
            sender_pseudonym,
            &channel_id,
            ciphertext,
            mek_generation,
        ),

        CommunityRequest::GetMessages {
            channel_id,
            before_timestamp,
            limit,
        } => handle_get_messages(
            state,
            &community_id,
            sender_pseudonym,
            &channel_id,
            before_timestamp,
            limit,
        ),

        CommunityRequest::RequestMEK => handle_request_mek(state, &community_id, sender_pseudonym),
        CommunityRequest::Leave => handle_leave(state, &community_id, sender_pseudonym).await,

        CommunityRequest::Kick { target_pseudonym } => {
            handle_kick(state, &community_id, sender_pseudonym, &target_pseudonym).await
        }

        CommunityRequest::CreateChannel { name, channel_type } => {
            let resp =
                handle_create_channel(state, &community_id, sender_pseudonym, &name, &channel_type);
            if matches!(resp, CommunityResponse::ChannelCreated { .. }) {
                let st = Arc::clone(state);
                let cid = community_id.clone();
                tokio::spawn(async move {
                    community_host::publish_channels(&st, &cid).await;
                });
            }
            resp
        }

        CommunityRequest::DeleteChannel { channel_id } => {
            let resp = handle_delete_channel(state, &community_id, sender_pseudonym, &channel_id);
            if matches!(resp, CommunityResponse::Ok) {
                let st = Arc::clone(state);
                let cid = community_id.clone();
                tokio::spawn(async move {
                    community_host::publish_channels(&st, &cid).await;
                });
            }
            resp
        }

        CommunityRequest::RotateMEK => {
            handle_rotate_mek(state, &community_id, sender_pseudonym).await
        }

        CommunityRequest::RenameChannel {
            channel_id,
            new_name,
        } => {
            let resp = handle_rename_channel(
                state,
                &community_id,
                sender_pseudonym,
                &channel_id,
                &new_name,
            );
            if matches!(resp, CommunityResponse::Ok) {
                let st = Arc::clone(state);
                let cid = community_id.clone();
                tokio::spawn(async move {
                    community_host::publish_channels(&st, &cid).await;
                });
            }
            resp
        }

        CommunityRequest::UpdateCommunity { name, description } => {
            handle_update_community(
                state,
                &community_id,
                sender_pseudonym,
                name.as_deref(),
                description.as_deref(),
            )
            .await
        }

        CommunityRequest::Ban { target_pseudonym } => {
            handle_ban(state, &community_id, sender_pseudonym, &target_pseudonym).await
        }

        CommunityRequest::Unban { target_pseudonym } => {
            handle_unban(state, &community_id, sender_pseudonym, &target_pseudonym)
        }

        CommunityRequest::GetBanList => handle_get_ban_list(state, &community_id, sender_pseudonym),

        // ── Role & permission management ──
        CommunityRequest::CreateRole {
            name,
            color,
            permissions: perms,
            hoist,
            mentionable,
        } => {
            let resp = handle_create_role(
                state,
                &community_id,
                sender_pseudonym,
                &name,
                color,
                perms,
                hoist,
                mentionable,
            );
            if let CommunityResponse::RoleCreated { .. } = &resp {
                broadcast_roles_changed(state, &community_id);
            }
            resp
        }

        CommunityRequest::EditRole {
            role_id,
            name,
            color,
            permissions: perms,
            position,
            hoist,
            mentionable,
        } => {
            let resp = handle_edit_role(
                state,
                &community_id,
                sender_pseudonym,
                role_id,
                name.as_ref(),
                color,
                perms,
                position,
                hoist,
                mentionable,
            );
            if matches!(resp, CommunityResponse::Ok) {
                broadcast_roles_changed(state, &community_id);
            }
            resp
        }

        CommunityRequest::DeleteRole { role_id } => {
            let resp = handle_delete_role(state, &community_id, sender_pseudonym, role_id);
            if matches!(resp, CommunityResponse::Ok) {
                broadcast_roles_changed(state, &community_id);
            }
            resp
        }

        CommunityRequest::AssignRole {
            target_pseudonym,
            role_id,
        } => {
            let resp = handle_assign_role(
                state,
                &community_id,
                sender_pseudonym,
                &target_pseudonym,
                role_id,
            );
            if matches!(resp, CommunityResponse::Ok) {
                broadcast_member_roles_changed(state, &community_id, &target_pseudonym);
            }
            resp
        }

        CommunityRequest::UnassignRole {
            target_pseudonym,
            role_id,
        } => {
            let resp = handle_unassign_role(
                state,
                &community_id,
                sender_pseudonym,
                &target_pseudonym,
                role_id,
            );
            if matches!(resp, CommunityResponse::Ok) {
                broadcast_member_roles_changed(state, &community_id, &target_pseudonym);
            }
            resp
        }

        CommunityRequest::SetChannelOverwrite {
            channel_id,
            target_type,
            target_id,
            allow,
            deny,
        } => handle_set_channel_overwrite(
            state,
            &community_id,
            sender_pseudonym,
            &channel_id,
            &target_type,
            &target_id,
            allow,
            deny,
        ),

        CommunityRequest::DeleteChannelOverwrite {
            channel_id,
            target_type,
            target_id,
        } => handle_delete_channel_overwrite(
            state,
            &community_id,
            sender_pseudonym,
            &channel_id,
            &target_type,
            &target_id,
        ),

        CommunityRequest::TimeoutMember {
            target_pseudonym,
            duration_seconds,
            reason,
        } => handle_timeout_member(
            state,
            &community_id,
            sender_pseudonym,
            &target_pseudonym,
            duration_seconds,
            reason.as_ref(),
        ),

        CommunityRequest::RemoveTimeout { target_pseudonym } => {
            handle_remove_timeout(state, &community_id, sender_pseudonym, &target_pseudonym)
        }

        CommunityRequest::GetRoles => handle_get_roles(state, &community_id, sender_pseudonym),
    }
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
        })
        .collect();

    let role_ids = community
        .members
        .iter()
        .find(|m| m.pseudonym_key_hex == pseudonym_pubkey)
        .map_or_else(Vec::new, |m| m.role_ids.clone());

    let mut mek_payload = Vec::with_capacity(40);
    mek_payload.extend_from_slice(&community.mek.generation().to_le_bytes());
    mek_payload.extend_from_slice(community.mek.as_bytes());

    CommunityResponse::Joined {
        mek_encrypted: mek_payload,
        mek_generation: community.mek.generation(),
        channels,
        role_ids,
        roles: roles_to_dto(community),
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
    let member = community
        .members
        .iter_mut()
        .find(|m| m.pseudonym_key_hex == pseudonym_pubkey)?;

    if let Some(blob) = member_route_blob {
        member.route_blob = Some(blob.to_vec());
        if let Ok(db) = state.db.lock() {
            let _ = db.execute(
                "UPDATE server_members SET route_blob = ? WHERE community_id = ? AND pseudonym_key_hex = ?",
                params![blob, community_id, pseudonym_pubkey],
            );
        }
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
    let now = timestamp_now();

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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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
    });

    if let Some(blob) = member_route_blob {
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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
        })
        .collect();

    let roles_dto = roles_to_dto(community);

    let mek_generation = community.mek.generation();
    let mut mek_payload = Vec::with_capacity(40);
    mek_payload.extend_from_slice(&mek_generation.to_le_bytes());
    mek_payload.extend_from_slice(community.mek.as_bytes());

    Some((
        mek_payload,
        mek_generation,
        channels,
        default_role_ids,
        roles_dto,
    ))
}

async fn handle_join(
    state: &Arc<ServerState>,
    pseudonym_pubkey: &str,
    display_name: &str,
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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

    if let Some(resp) = handle_rejoin(
        state,
        &community_id,
        pseudonym_pubkey,
        member_route_blob.as_deref(),
    ) {
        return resp;
    }

    let Some((mek_payload, mek_generation, channels, role_ids, roles)) = add_new_member(
        state,
        &community_id,
        pseudonym_pubkey,
        display_name,
        member_route_blob.as_deref(),
    ) else {
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

    CommunityResponse::Joined {
        mek_encrypted: mek_payload,
        mek_generation,
        channels,
        role_ids,
        roles,
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
        if let Some(member) = community
            .members
            .iter()
            .find(|m| m.pseudonym_key_hex == sender_pseudonym)
        {
            let ch_overwrites = community
                .channels
                .iter()
                .find(|ch| ch.id == channel_id)
                .map_or(&[][..], |ch| &ch.permission_overwrites);
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
    }

    let now = timestamp_now();

    {
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        let mek_gen_i64 = i64::try_from(mek_generation).unwrap_or(i64::MAX);
        if let Err(e) = db.execute(
            "INSERT INTO server_messages (community_id, channel_id, sender_pseudonym, ciphertext, mek_generation, timestamp) VALUES (?,?,?,?,?,?)",
            params![community_id, channel_id, sender_pseudonym, ciphertext, mek_gen_i64, now],
        ) {
            tracing::error!(error = %e, "failed to store message in DB");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to store message".into(),
            };
        }
    }

    let now_u64: u64 = now.try_into().unwrap_or(0u64);
    broadcast_to_members(
        state,
        community_id,
        sender_pseudonym,
        &CommunityBroadcast::NewMessage {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            sender_pseudonym: sender_pseudonym.to_string(),
            ciphertext,
            mek_generation,
            timestamp: now_u64,
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

    let db = state.db.lock().unwrap_or_else(|e| {
        tracing::error!(error = %e, "server db mutex poisoned — recovering");
        e.into_inner()
    });

    let query_result: Result<Vec<ChannelMessageDto>, _> = if let Some(before) = before_timestamp {
        let before_i64: i64 = before.try_into().unwrap_or(i64::MAX);
        db.prepare(
            "SELECT sender_pseudonym, ciphertext, mek_generation, timestamp FROM server_messages \
             WHERE community_id = ? AND channel_id = ? AND timestamp < ? \
             ORDER BY timestamp DESC LIMIT ?",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(
                params![community_id, channel_id, before_i64, limit],
                |row| {
                    let mek_gen: i64 = row.get(2)?;
                    let ts: i64 = row.get(3)?;
                    Ok(ChannelMessageDto {
                        sender_pseudonym: row.get(0)?,
                        ciphertext: row.get(1)?,
                        mek_generation: mek_gen.try_into().unwrap_or(0u64),
                        timestamp: ts.try_into().unwrap_or(0u64),
                    })
                },
            )?;
            rows.collect()
        })
    } else {
        db.prepare(
            "SELECT sender_pseudonym, ciphertext, mek_generation, timestamp FROM server_messages \
             WHERE community_id = ? AND channel_id = ? \
             ORDER BY timestamp DESC LIMIT ?",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![community_id, channel_id, limit], |row| {
                let mek_gen: i64 = row.get(2)?;
                let ts: i64 = row.get(3)?;
                Ok(ChannelMessageDto {
                    sender_pseudonym: row.get(0)?,
                    ciphertext: row.get(1)?,
                    mek_generation: mek_gen.try_into().unwrap_or(0u64),
                    timestamp: ts.try_into().unwrap_or(0u64),
                })
            })?;
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

    messages.reverse();
    CommunityResponse::Messages { messages }
}

fn handle_request_mek(
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

    let mut mek_payload = Vec::with_capacity(40);
    mek_payload.extend_from_slice(&community.mek.generation().to_le_bytes());
    mek_payload.extend_from_slice(community.mek.as_bytes());

    CommunityResponse::MEK {
        mek_encrypted: mek_payload,
        mek_generation: community.mek.generation(),
    }
}

// ---------------------------------------------------------------------------
// Leave / Kick
// ---------------------------------------------------------------------------

async fn handle_leave(
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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

async fn handle_kick(
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

    handle_leave(state, community_id, target_pseudonym).await
}

// ---------------------------------------------------------------------------
// Channel management
// ---------------------------------------------------------------------------

fn handle_create_channel(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    name: &str,
    channel_type: &str,
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

    if channel_type != "text" && channel_type != "voice" {
        return CommunityResponse::Error {
            code: 400,
            message: format!("invalid channel type '{channel_type}': must be 'text' or 'voice'"),
        };
    }

    let channel_id = format!("channel_{}", hex::encode(rand_bytes(8)));
    let sort_order = i32::try_from(community.channels.len()).unwrap_or(i32::MAX);
    let channel = ServerChannel {
        id: channel_id.clone(),
        name: name.to_string(),
        channel_type: channel_type.to_string(),
        sort_order,
        permission_overwrites: Vec::new(),
    };

    {
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        if let Err(e) = db.execute(
            "INSERT INTO server_channels (community_id, id, name, channel_type, sort_order) VALUES (?,?,?,?,?)",
            params![community.community_id, channel.id, channel.name, channel.channel_type, channel.sort_order],
        ) {
            tracing::error!(error = %e, "failed to insert channel into DB");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to create channel".into(),
            };
        }
    }

    community.channels.push(channel);
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        if let Err(e) = db.execute(
            "DELETE FROM server_channels WHERE community_id = ? AND id = ?",
            params![community.community_id, channel_id],
        ) {
            tracing::error!(error = %e, "failed to delete channel from DB");
        }
    }

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

    let Some(channel) = community.channels.iter_mut().find(|ch| ch.id == channel_id) else {
        return CommunityResponse::Error {
            code: 404,
            message: "channel not found".into(),
        };
    };

    channel.name = new_name.to_string();

    {
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        if let Err(e) = db.execute(
            "UPDATE server_channels SET name = ? WHERE community_id = ? AND id = ?",
            params![new_name, community.community_id, channel_id],
        ) {
            tracing::error!(error = %e, "failed to rename channel in DB");
        }
    }

    CommunityResponse::Ok
}

// ---------------------------------------------------------------------------
// MEK rotation
// ---------------------------------------------------------------------------

async fn handle_rotate_mek(
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

    CommunityResponse::Ok
}

// ---------------------------------------------------------------------------
// Community metadata update
// ---------------------------------------------------------------------------

async fn handle_update_community(
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
            let db = state.db.lock().unwrap_or_else(|e| {
                tracing::error!(error = %e, "server db mutex poisoned — recovering");
                e.into_inner()
            });
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

    CommunityResponse::CommunityUpdated
}

// ---------------------------------------------------------------------------
// Ban / Unban
// ---------------------------------------------------------------------------

async fn handle_ban(
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
            .members
            .iter()
            .find(|m| m.pseudonym_key_hex == target_pseudonym)
            .map_or_else(String::new, |m| m.display_name.clone())
    };

    {
        let now = timestamp_now();
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        if let Err(e) = db.execute(
            "INSERT OR REPLACE INTO banned_members (community_id, pseudonym_key_hex, display_name, banned_at) VALUES (?,?,?,?)",
            params![community_id, target_pseudonym, display_name, now],
        ) {
            tracing::error!(error = %e, "failed to insert ban record");
        }
    }

    handle_leave(state, community_id, target_pseudonym).await
}

fn handle_unban(
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        if let Err(e) = db.execute(
            "DELETE FROM banned_members WHERE community_id = ? AND pseudonym_key_hex = ?",
            params![community.community_id, target_pseudonym],
        ) {
            tracing::error!(error = %e, "failed to remove ban record");
        }
    }

    tracing::info!(community = %community.community_id, member = %target_pseudonym, "member unbanned");
    CommunityResponse::Ok
}

fn handle_get_ban_list(
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

    let db = state.db.lock().unwrap_or_else(|e| {
        tracing::error!(error = %e, "server db mutex poisoned — recovering");
        e.into_inner()
    });

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
// Role management
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_create_role(
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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
    CommunityResponse::RoleCreated { role_id: next_id }
}

#[allow(clippy::too_many_arguments)]
fn handle_edit_role(
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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

    CommunityResponse::Ok
}

fn handle_delete_role(
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        let _ = db.execute(
            "DELETE FROM server_roles WHERE community_id = ? AND id = ?",
            params![community.community_id, role_id],
        );
        let _ = db.execute(
            "DELETE FROM server_member_roles WHERE community_id = ? AND role_id = ?",
            params![community.community_id, role_id],
        );
    }

    CommunityResponse::Ok
}

fn handle_assign_role(
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

    let Some(member) = community
        .members
        .iter_mut()
        .find(|m| m.pseudonym_key_hex == target_pseudonym)
    else {
        return CommunityResponse::Error {
            code: 404,
            message: "target member not found".into(),
        };
    };

    if !member.role_ids.contains(&role_id) {
        member.role_ids.push(role_id);
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        let _ = db.execute(
            "INSERT OR IGNORE INTO server_member_roles (community_id, pseudonym_key_hex, role_id) VALUES (?,?,?)",
            params![community.community_id, target_pseudonym, role_id],
        );
    }

    CommunityResponse::Ok
}

fn handle_unassign_role(
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

    let Some(member) = community
        .members
        .iter_mut()
        .find(|m| m.pseudonym_key_hex == target_pseudonym)
    else {
        return CommunityResponse::Error {
            code: 404,
            message: "target member not found".into(),
        };
    };

    member.role_ids.retain(|rid| *rid != role_id);

    {
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        let _ = db.execute(
            "DELETE FROM server_member_roles WHERE community_id = ? AND pseudonym_key_hex = ? AND role_id = ?",
            params![community.community_id, target_pseudonym, role_id],
        );
    }

    CommunityResponse::Ok
}

fn handle_get_roles(
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

#[allow(clippy::too_many_arguments)]
fn handle_set_channel_overwrite(
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

    let Some(channel) = community.channels.iter_mut().find(|ch| ch.id == channel_id) else {
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
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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

fn handle_delete_channel_overwrite(
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

    if let Some(channel) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
        channel
            .permission_overwrites
            .retain(|o| !(o.target_type == ow_type && o.target_id == target_id));
    }

    {
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
        let _ = db.execute(
            "DELETE FROM server_channel_overwrites WHERE community_id = ? AND channel_id = ? AND target_type = ? AND target_id = ?",
            params![community.community_id, channel_id, target_type, target_id],
        );
    }

    CommunityResponse::Ok
}

// ---------------------------------------------------------------------------
// Timeouts
// ---------------------------------------------------------------------------

fn handle_timeout_member(
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

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let timeout_until = now + duration_seconds;

    let Some(member) = community
        .members
        .iter_mut()
        .find(|m| m.pseudonym_key_hex == target_pseudonym)
    else {
        return CommunityResponse::Error {
            code: 404,
            message: "target member not found".into(),
        };
    };

    member.timeout_until = Some(timeout_until);

    {
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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

    CommunityResponse::Ok
}

fn handle_remove_timeout(
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

    if let Some(member) = community
        .members
        .iter_mut()
        .find(|m| m.pseudonym_key_hex == target_pseudonym)
    {
        member.timeout_until = None;
    }

    {
        let db = state.db.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "server db mutex poisoned — recovering");
            e.into_inner()
        });
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

    CommunityResponse::Ok
}

// ---------------------------------------------------------------------------
// Broadcast helpers
// ---------------------------------------------------------------------------

fn broadcast_roles_changed(state: &Arc<ServerState>, community_id: &str) {
    let roles = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return;
        };
        roles_to_dto(community)
    };

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::RolesChanged {
            community_id: community_id.to_string(),
            roles,
        },
    );
}

fn broadcast_member_roles_changed(
    state: &Arc<ServerState>,
    community_id: &str,
    target_pseudonym: &str,
) {
    let role_ids = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return;
        };
        community
            .members
            .iter()
            .find(|m| m.pseudonym_key_hex == target_pseudonym)
            .map_or_else(Vec::new, |m| m.role_ids.clone())
    };

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::MemberRolesChanged {
            community_id: community_id.to_string(),
            pseudonym_key: target_pseudonym.to_string(),
            role_ids,
        },
    );
}

fn broadcast_to_members(
    state: &Arc<ServerState>,
    community_id: &str,
    exclude_pseudonym: &str,
    broadcast: &CommunityBroadcast,
) {
    let broadcast_bytes = serde_json::to_vec(broadcast).unwrap_or_default();

    let member_routes: Vec<Vec<u8>> = {
        let hosted = state.hosted.read();
        hosted
            .get(community_id)
            .map(|c| {
                c.members
                    .iter()
                    .filter(|m| m.pseudonym_key_hex != exclude_pseudonym)
                    .filter_map(|m| m.route_blob.clone())
                    .collect()
            })
            .unwrap_or_default()
    };

    for route_blob in member_routes {
        let api = state.api.clone();
        let rc = state.routing_context.clone();
        let data = broadcast_bytes.clone();
        tokio::spawn(async move {
            match api.import_remote_private_route(route_blob) {
                Ok(route_id) => {
                    if let Err(e) = rc
                        .app_message(veilid_core::Target::RouteId(route_id), data)
                        .await
                    {
                        tracing::debug!(error = %e, "failed to broadcast message to member");
                    }
                }
                Err(e) => {
                    tracing::debug!(error = %e, "failed to import member route for broadcast");
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn timestamp_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn rand_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}
