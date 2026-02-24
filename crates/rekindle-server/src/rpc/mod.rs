mod audit_log;
mod broadcast;
mod channels;
mod events;
mod game_servers;
mod invites;
mod membership;
mod messages;
mod permissions;
mod presence;
mod reactions;
mod roles;
mod threads;
mod unread;

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::{CommunityRequest, CommunityResponse};
use rekindle_protocol::messaging::receiver::process_incoming;

use crate::server_state::ServerState;

// Re-export the one pub function that main.rs needs
pub use broadcast::broadcast_event_reminder;

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
        invite_code,
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
        return membership::handle_join(
            state,
            sender_pseudonym,
            &display_name,
            invite_code.as_deref(),
            route_blob,
            incoming_route_id,
            ipc_community_id,
        )
        .await;
    }

    // For all non-Join requests, resolve the target community upfront.
    let Some(community_id) =
        permissions::resolve_community_id(state, sender_pseudonym, incoming_route_id, ipc_community_id)
    else {
        return CommunityResponse::Error {
            code: 403,
            message: "not a member of any hosted community".into(),
        };
    };

    match request {
        CommunityRequest::Join { .. } => unreachable!(),

        // Message operations (send, edit, delete, get, reactions, pins)
        CommunityRequest::SendMessage { .. }
        | CommunityRequest::EditMessage { .. }
        | CommunityRequest::DeleteMessage { .. }
        | CommunityRequest::GetMessages { .. }
        | CommunityRequest::AddReaction { .. }
        | CommunityRequest::RemoveReaction { .. }
        | CommunityRequest::PinMessage { .. }
        | CommunityRequest::UnpinMessage { .. }
        | CommunityRequest::GetPins { .. } => {
            messages::dispatch_message_request(state, &community_id, sender_pseudonym, request)
        }

        CommunityRequest::RequestMEK => membership::handle_request_mek(state, &community_id, sender_pseudonym),
        CommunityRequest::Leave => membership::handle_leave(state, &community_id, sender_pseudonym).await,

        CommunityRequest::Kick { target_pseudonym } => {
            membership::handle_kick(state, &community_id, sender_pseudonym, &target_pseudonym).await
        }

        // Channel & category operations (dispatch to helper to keep this fn under 300 lines)
        CommunityRequest::CreateChannel { .. }
        | CommunityRequest::DeleteChannel { .. }
        | CommunityRequest::RenameChannel { .. }
        | CommunityRequest::CreateCategory { .. }
        | CommunityRequest::DeleteCategory { .. }
        | CommunityRequest::RenameCategory { .. }
        | CommunityRequest::MoveChannel { .. }
        | CommunityRequest::ReorderCategories { .. }
        | CommunityRequest::SetChannelTopic { .. }
        | CommunityRequest::ReorderChannels { .. }
        | CommunityRequest::SetSlowmode { .. } => {
            channels::dispatch_channel_request(state, &community_id, sender_pseudonym, request)
        }

        CommunityRequest::RotateMEK => {
            membership::handle_rotate_mek(state, &community_id, sender_pseudonym).await
        }

        CommunityRequest::UpdateCommunity { name, description } => {
            membership::handle_update_community(
                state,
                &community_id,
                sender_pseudonym,
                name.as_deref(),
                description.as_deref(),
            )
            .await
        }

        CommunityRequest::Ban { target_pseudonym } => {
            membership::handle_ban(state, &community_id, sender_pseudonym, &target_pseudonym).await
        }

        CommunityRequest::Unban { target_pseudonym } => {
            membership::handle_unban(state, &community_id, sender_pseudonym, &target_pseudonym)
        }

        CommunityRequest::GetBanList => membership::handle_get_ban_list(state, &community_id, sender_pseudonym),

        // ── Role & permission management ──
        CommunityRequest::CreateRole {
            name,
            color,
            permissions: perms,
            hoist,
            mentionable,
        } => {
            let resp = roles::handle_create_role(
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
                broadcast::broadcast_roles_changed(state, &community_id);
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
            let resp = roles::handle_edit_role(
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
                broadcast::broadcast_roles_changed(state, &community_id);
            }
            resp
        }

        CommunityRequest::DeleteRole { role_id } => {
            let resp = roles::handle_delete_role(state, &community_id, sender_pseudonym, role_id);
            if matches!(resp, CommunityResponse::Ok) {
                broadcast::broadcast_roles_changed(state, &community_id);
            }
            resp
        }

        CommunityRequest::AssignRole {
            target_pseudonym,
            role_id,
        } => {
            let resp = roles::handle_assign_role(
                state,
                &community_id,
                sender_pseudonym,
                &target_pseudonym,
                role_id,
            );
            if matches!(resp, CommunityResponse::Ok) {
                broadcast::broadcast_member_roles_changed(state, &community_id, &target_pseudonym);
            }
            resp
        }

        CommunityRequest::UnassignRole {
            target_pseudonym,
            role_id,
        } => {
            let resp = roles::handle_unassign_role(
                state,
                &community_id,
                sender_pseudonym,
                &target_pseudonym,
                role_id,
            );
            if matches!(resp, CommunityResponse::Ok) {
                broadcast::broadcast_member_roles_changed(state, &community_id, &target_pseudonym);
            }
            resp
        }

        CommunityRequest::SetChannelOverwrite {
            channel_id,
            target_type,
            target_id,
            allow,
            deny,
        } => roles::handle_set_channel_overwrite(
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
        } => roles::handle_delete_channel_overwrite(
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
        } => membership::handle_timeout_member(
            state,
            &community_id,
            sender_pseudonym,
            &target_pseudonym,
            duration_seconds,
            reason.as_ref(),
        ),

        CommunityRequest::RemoveTimeout { target_pseudonym } => {
            membership::handle_remove_timeout(state, &community_id, sender_pseudonym, &target_pseudonym)
        }

        CommunityRequest::GetRoles => roles::handle_get_roles(state, &community_id, sender_pseudonym),

        // ── Invite management ──
        CommunityRequest::CreateInvite {
            max_uses,
            expires_in_seconds,
        } => invites::handle_create_invite(
            state,
            &community_id,
            sender_pseudonym,
            max_uses,
            expires_in_seconds,
        ),

        CommunityRequest::RevokeInvite { code } => {
            invites::handle_revoke_invite(state, &community_id, sender_pseudonym, &code)
        }

        CommunityRequest::ListInvites => {
            invites::handle_list_invites(state, &community_id, sender_pseudonym)
        }

        CommunityRequest::GetAuditLog {
            before_timestamp,
            limit,
        } => audit_log::handle_get_audit_log(state, &community_id, sender_pseudonym, before_timestamp, limit),

        CommunityRequest::ChannelTyping { channel_id } => {
            presence::handle_channel_typing(state, &community_id, sender_pseudonym, &channel_id)
        }

        CommunityRequest::UpdatePresence { status, game_name, game_id, elapsed_seconds, server_address } => {
            presence::handle_update_presence(state, &community_id, sender_pseudonym, &status, game_name, game_id, elapsed_seconds, server_address)
        }

        // ── Event operations ──
        CommunityRequest::CreateEvent { .. }
        | CommunityRequest::EditEvent { .. }
        | CommunityRequest::DeleteEvent { .. }
        | CommunityRequest::CancelEvent { .. }
        | CommunityRequest::RsvpEvent { .. }
        | CommunityRequest::GetEvents => {
            dispatch_event_request(state, &community_id, sender_pseudonym, request)
        }

        // ── Thread operations ──
        CommunityRequest::CreateThread { .. }
        | CommunityRequest::GetChannelThreads { .. }
        | CommunityRequest::SendThreadMessage { .. }
        | CommunityRequest::GetThreadMessages { .. }
        | CommunityRequest::ArchiveThread { .. }
        | CommunityRequest::UnarchiveThread { .. } => {
            threads::dispatch_thread_request(state, &community_id, sender_pseudonym, request)
        }

        // ── Game server favorites ──
        CommunityRequest::AddGameServer {
            game_id,
            label,
            address,
        } => game_servers::handle_add_game_server(
            state,
            &community_id,
            sender_pseudonym,
            &game_id,
            &label,
            &address,
        ),

        CommunityRequest::RemoveGameServer { server_id } => {
            game_servers::handle_remove_game_server(state, &community_id, sender_pseudonym, &server_id)
        }

        CommunityRequest::GetGameServers => {
            game_servers::handle_get_game_servers(state, &community_id, sender_pseudonym)
        }

        // ── Unread tracking ──
        CommunityRequest::MarkChannelRead {
            channel_id,
            last_message_id,
        } => unread::handle_mark_channel_read(
            state,
            &community_id,
            sender_pseudonym,
            &channel_id,
            &last_message_id,
        ),

        CommunityRequest::GetUnreadCounts => {
            unread::handle_get_unread_counts(state, &community_id, sender_pseudonym)
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-dispatchers (to keep dispatch_request under the 300-line limit)
// ---------------------------------------------------------------------------

fn dispatch_event_request(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    request: CommunityRequest,
) -> CommunityResponse {
    match request {
        CommunityRequest::CreateEvent { title, description, start_time, end_time, channel_id, max_attendees } => {
            events::handle_create_event(state, community_id, sender_pseudonym, &title, &description, start_time, end_time, channel_id.as_deref(), max_attendees)
        }
        CommunityRequest::EditEvent { event_id, title, description, start_time, end_time, channel_id, max_attendees } => {
            events::handle_edit_event(state, community_id, sender_pseudonym, &event_id, title.as_deref(), description.as_deref(), start_time, end_time, channel_id.as_deref(), max_attendees)
        }
        CommunityRequest::DeleteEvent { event_id } => {
            events::handle_delete_event(state, community_id, sender_pseudonym, &event_id)
        }
        CommunityRequest::CancelEvent { event_id } => {
            events::handle_cancel_event(state, community_id, sender_pseudonym, &event_id)
        }
        CommunityRequest::RsvpEvent { event_id, status } => {
            events::handle_rsvp_event(state, community_id, sender_pseudonym, &event_id, &status)
        }
        CommunityRequest::GetEvents => {
            events::handle_get_events(state, community_id, sender_pseudonym)
        }
        _ => CommunityResponse::Error {
            code: 400,
            message: "invalid event request".into(),
        },
    }
}

// ---------------------------------------------------------------------------
// Shared utilities
// ---------------------------------------------------------------------------

fn rand_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}
