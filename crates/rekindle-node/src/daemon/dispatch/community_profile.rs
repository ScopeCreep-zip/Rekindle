//! Our per-community profile, the community's pictures and its policy
//! (plan E3.1 D4), answered by the Host's governance adapter on both hosts.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::SettingsRequest;
use rekindle_types::image_kind::ImageKind;

pub(crate) use crate::host::adapters::governance::community_pictures::PictureSlot;
pub(crate) use crate::host::adapters::governance::community_profile::{PolicyEdit, ProfileEdit};

use super::{adapter, runtime_refused, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Make `edit` our profile in `community`.
pub(crate) async fn handle_update_profile(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    edit: ProfileEdit,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).update_profile(&membership.anchor, edit).await {
        Ok(()) => IpcResponse::ok(&()),
        Err(e) => IpcResponse::error(404, format!("community profile: {e}")),
    }
}

/// Make `name` our name in `community` (plan E3.4 S6).
pub(crate) async fn handle_set_display_name(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    name: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .set_display_name(&membership.anchor, name)
        .await
    {
        Ok(()) => IpcResponse::ok(&()),
        Err(e) => IpcResponse::error(400, format!("community name: {e}")),
    }
}

/// Make `policy` our presence sharing consent in the community.
async fn handle_set_presence_policy(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    update: impl FnOnce(&mut rekindle_types::presence::PresenceSharingPolicy) + Send + 'static,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .edit_presence_policy(&membership.anchor, update)
        .await
    {
        Ok(()) => IpcResponse::ok(&()),
        Err(e) => IpcResponse::error(500, format!("presence policy: {e}")),
    }
}

/// Cache an uploaded avatar or banner; answered with its hash.
pub(crate) async fn handle_set_picture(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    image: Vec<u8>,
    kind: ImageKind,
    slot: PictureSlot,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .set_community_picture(&membership.anchor, image, kind, slot)
        .await
    {
        Ok(hash) => IpcResponse::ok(&hash),
        Err(e) => IpcResponse::error(400, format!("community picture: {e}")),
    }
}

/// A cached picture as a data URL, or `None`.
pub(crate) async fn handle_picture(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    hash: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .community_picture_data_url(&membership.anchor, hash)
        .await
    {
        Ok(url) => IpcResponse::ok(&url),
        Err(e) => IpcResponse::error(500, format!("community picture: {e}")),
    }
}

/// The community's policy.
pub(crate) fn handle_policy(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).community_policy(&membership.anchor) {
        Ok(policy) => IpcResponse::ok(&policy),
        Err(e) => runtime_refused(&e, "community policy"),
    }
}

/// Write a new policy.
pub(crate) async fn handle_set_policy(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    edit: PolicyEdit,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .set_community_policy(&membership.anchor, edit)
        .await
    {
        Ok(()) => IpcResponse::ok(&()),
        Err(e) => runtime_refused(&e, "community policy"),
    }
}

/// Route a profile, picture or policy request (the community router's
/// profile arm) to its handler.
pub(crate) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: SettingsRequest,
) -> IpcResponse {
    match request {
        SettingsRequest::CommunityUpdateProfile {
            community,
            bio,
            pronouns,
            theme_color,
            badges,
            avatar_ref,
            banner_ref,
            custom_status,
        } => {
            let edit = ProfileEdit {
                bio,
                pronouns,
                theme_color,
                badges,
                avatar_ref,
                banner_ref,
                custom_status,
            };
            handle_update_profile(ctx, state, &community, edit).await
        }
        SettingsRequest::CommunitySetDisplayName {
            community,
            display_name,
        } => handle_set_display_name(ctx, state, &community, &display_name).await,
        SettingsRequest::CommunitySetPresencePolicy { community, policy } => {
            handle_set_presence_policy(ctx, state, &community, move |current| *current = policy)
                .await
        }
        SettingsRequest::CommunitySetPresencePolicyField { community, field } => {
            handle_set_presence_policy(ctx, state, &community, move |policy| field.apply(policy))
                .await
        }
        SettingsRequest::CommunityGetPresencePolicy { community } => {
            if !state.can_query() {
                return state_error(state, "query");
            }
            let membership = match ctx.resolve_community(&community) {
                Ok(membership) => membership,
                Err(error) => return error,
            };
            crate::host::adapters::presence::stored_policy(ctx, &membership.anchor).map_or_else(
                || IpcResponse::error(404, "community presence policy not found"),
                |policy| IpcResponse::ok(&policy),
            )
        }
        SettingsRequest::CommunitySetAvatar {
            community,
            image,
            kind,
        } => {
            let slot = PictureSlot::Avatar;
            handle_set_picture(ctx, state, &community, image, kind, slot).await
        }
        SettingsRequest::CommunitySetBanner {
            community,
            image,
            kind,
        } => {
            let slot = PictureSlot::Banner;
            handle_set_picture(ctx, state, &community, image, kind, slot).await
        }
        SettingsRequest::CommunityAvatarGet { community, hash } => {
            handle_picture(ctx, state, &community, &hash).await
        }
        SettingsRequest::CommunityPolicyGet { community } => handle_policy(ctx, state, &community),
        SettingsRequest::CommunityPolicySet {
            community,
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
        } => {
            let edit = PolicyEdit {
                policy_text,
                max_joins_per_interval,
                join_interval_seconds,
            };
            handle_set_policy(ctx, state, &community, edit).await
        }
        other => unreachable!("{} is not a profile request", other.name()),
    }
}
