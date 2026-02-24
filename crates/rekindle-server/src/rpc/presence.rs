use std::sync::Arc;

use rekindle_protocol::messaging::envelope::{CommunityBroadcast, CommunityResponse};

use crate::server_state::ServerState;

use super::broadcast::broadcast_to_members;
use super::permissions::verify_membership;

pub(super) fn handle_channel_typing(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: &str,
) -> CommunityResponse {
    // Verify membership (no permission check needed — just membership)
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

    // Ephemeral broadcast — no DB storage
    broadcast_to_members(
        state,
        community_id,
        sender_pseudonym, // exclude sender from broadcast
        &CommunityBroadcast::ChannelTyping {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            pseudonym_key: sender_pseudonym.to_string(),
        },
    );

    CommunityResponse::Ok
}

pub(super) fn handle_update_presence(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    status: &str,
    game_name: Option<String>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<String>,
) -> CommunityResponse {
    // Verify membership and update in-memory status
    {
        let mut hosted = state.hosted.write();
        let Some(community) = hosted.get_mut(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        let Some(member) = community.find_member_mut(sender_pseudonym) else {
            return CommunityResponse::Error {
                code: 403,
                message: "not a member".into(),
            };
        };
        member.online_status = status.to_string();
    }

    broadcast_to_members(
        state,
        community_id,
        sender_pseudonym,
        &CommunityBroadcast::MemberPresenceChanged {
            community_id: community_id.to_string(),
            pseudonym_key: sender_pseudonym.to_string(),
            status: status.to_string(),
            game_name,
            game_id,
            elapsed_seconds,
            server_address,
        },
    );

    CommunityResponse::Ok
}
