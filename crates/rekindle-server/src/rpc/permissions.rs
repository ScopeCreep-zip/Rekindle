use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{CategoryDto, CommunityResponse, RoleDto};

use crate::server_state::{HostedCommunity, ServerState};

// ---------------------------------------------------------------------------
// Permission helpers
// ---------------------------------------------------------------------------

/// Calculate a member's effective base permissions from their roles.
pub(super) fn member_base_permissions(community: &HostedCommunity, pseudonym: &str) -> u64 {
    let Some(member) = community.find_member(pseudonym) else {
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
pub(super) fn check_permission(
    community: &HostedCommunity,
    sender_pseudonym: &str,
    required: u64,
) -> Result<(), CommunityResponse> {
    // Community creator always has all permissions
    if community.is_creator(sender_pseudonym) {
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
pub(super) fn highest_role_position(community: &HostedCommunity, pseudonym: &str) -> i32 {
    let Some(member) = community.find_member(pseudonym) else {
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
pub(super) fn check_hierarchy(
    community: &HostedCommunity,
    sender_pseudonym: &str,
    target_pseudonym: &str,
) -> Result<(), CommunityResponse> {
    // Community creator always outranks everyone
    if community.is_creator(sender_pseudonym) {
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
pub(super) fn roles_to_dto(community: &HostedCommunity) -> Vec<RoleDto> {
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

/// Build `CategoryDto` vec from community categories.
pub(super) fn categories_to_dto(community: &HostedCommunity) -> Vec<CategoryDto> {
    community
        .categories
        .iter()
        .map(|c| CategoryDto {
            id: c.id.clone(),
            name: c.name.clone(),
            sort_order: c.sort_order,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Routing helpers
// ---------------------------------------------------------------------------

/// Find which hosted community a member belongs to by their pseudonym key.
pub(super) fn find_community_for_member(
    state: &Arc<ServerState>,
    pseudonym: &str,
) -> Option<String> {
    let hosted = state.hosted.read();
    hosted
        .values()
        .find(|c| c.is_member(pseudonym))
        .map(|c| c.community_id.clone())
}

/// Find which hosted community a route belongs to by its private route ID.
///
/// Checks both the current route and any recently-rotated routes still within
/// their grace period (see `previous_route_ids`).
pub(super) fn find_community_for_route(
    state: &Arc<ServerState>,
    route_id: &veilid_core::RouteId,
) -> Option<String> {
    let hosted = state.hosted.read();
    let now = rekindle_utils::timestamp_secs();
    hosted
        .values()
        .find(|c| {
            // Check current route
            c.route_id.as_ref() == Some(route_id)
            // Check grace-period routes (not yet expired)
            || c.previous_route_ids
                .iter()
                .any(|(rid, exp)| rid == route_id && *exp > now)
        })
        .map(|c| c.community_id.clone())
}

/// Resolve the target community ID for a non-Join request.
///
/// Priority: IPC-provided `community_id` > route-based lookup > member scan.
pub(super) fn resolve_community_id(
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
pub(super) fn verify_membership(
    community: &HostedCommunity,
    sender_pseudonym: &str,
) -> Result<(), CommunityResponse> {
    if community.is_creator(sender_pseudonym) {
        return Ok(());
    }
    if community.is_member(sender_pseudonym) {
        return Ok(());
    }
    Err(CommunityResponse::Error {
        code: 403,
        message: "not a member".into(),
    })
}
