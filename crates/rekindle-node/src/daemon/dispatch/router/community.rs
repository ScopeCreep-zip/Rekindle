//! The community arm of the `dispatch()` router (plan E3.1): a community,
//! channel, role, moderation, invite or channel-key request to its domain
//! handler.

use rekindle_ipc::protocol::{
    ActivityRequest, AuditRequest, ChannelMessageRequest, ChannelRequest, ExpressionRequest,
    LifecycleRequest, MemberRequest, PinRequest, RichRequest, SettingsRequest, UnreadRequest,
};
use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use crate::daemon::dispatch::{
    audit, automod, channel, channel_message, channel_message_ops, community, community_profile,
    events, expressions, game_servers, governance, keys, member_block, onboarding, pins,
    rich_messaging, unread, DaemonContext,
};
use crate::daemon::DaemonState;

/// Route a community request: one arm per variant.
pub(super) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    match request {
        CommunityRequest::Lifecycle(request) => route_lifecycle(ctx, state, request).await,
        CommunityRequest::Settings(request) => route_settings(ctx, state, request).await,
        CommunityRequest::Channels(request) => route_channels(ctx, state, request).await,
        CommunityRequest::Messages(request) => route_messages(ctx, state, request).await,
        CommunityRequest::Unread(request) => route_unread(ctx, state, request).await,
        CommunityRequest::Audit(request) => route_audit(ctx, state, request).await,
        CommunityRequest::Activities(request) => route_activities(ctx, state, request).await,
        CommunityRequest::Members(request) => route_members(ctx, state, request).await,
        CommunityRequest::Pins(request) => route_pins(ctx, state, request).await,
        CommunityRequest::Rich(request) => route_rich(ctx, state, request).await,
        CommunityRequest::Expressions(request) => route_expressions(ctx, state, request).await,
    }
}

/// `route` for the community lifecycle and invite requests.
pub(super) async fn route_lifecycle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: LifecycleRequest,
) -> IpcResponse {
    match request {
        // ── Community ────────────────────────────────────────────
        LifecycleRequest::CommunityCreate {
            name,
            description,
            approval_required,
        } => community::handle_create(ctx, state, &name, &description, approval_required).await,
        LifecycleRequest::CommunityJoin { invite } => {
            community::handle_join(ctx, state, &invite).await
        }
        LifecycleRequest::CommunityLeave { governance_key } => {
            community::handle_leave(ctx, state, &governance_key).await
        }
        LifecycleRequest::CommunityList => community::handle_list(ctx, state),
        LifecycleRequest::CommunityViews => community::handle_views(ctx, state),
        LifecycleRequest::CommunityMembers { governance_key } => {
            community::handle_members(ctx, state, &governance_key).await
        }
        LifecycleRequest::CommunityInfo { governance_key } => {
            community::handle_info(ctx, state, &governance_key)
        }
        LifecycleRequest::CommunityUpdateInfo {
            governance_key,
            name,
            description,
            icon_hash,
            banner_hash,
        } => {
            community::handle_update_info(
                ctx,
                state,
                &governance_key,
                crate::host::adapters::governance::InfoEdit {
                    name,
                    description,
                    icon_hash,
                    banner_hash,
                },
            )
            .await
        }
        LifecycleRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        } => community::handle_approve(ctx, state, &governance_key, &member_pseudonym).await,
        LifecycleRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            reason,
        } => {
            community::handle_reject(ctx, state, &governance_key, &member_pseudonym, &reason).await
        }
        LifecycleRequest::CommunityPendingMembers { governance_key } => {
            community::handle_pending_members(ctx, state, &governance_key)
        }
        LifecycleRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym,
        } => {
            community::handle_transfer_ownership(ctx, state, &governance_key, &new_owner_pseudonym)
                .await
        }
        LifecycleRequest::InviteCreate {
            community,
            max_uses,
            expires_seconds,
        } => {
            governance::handle_invite_create(ctx, state, &community, max_uses, expires_seconds)
                .await
        }
        LifecycleRequest::InviteList { community } => {
            governance::handle_invite_list(ctx, state, &community).await
        }
        LifecycleRequest::InviteRevoke {
            community,
            invite_code,
        } => governance::handle_invite_revoke(ctx, state, &community, &invite_code).await,
    }
}

/// `route` for the profile, policy and onboarding requests.
pub(super) async fn route_settings(
    ctx: &DaemonContext,
    state: DaemonState,
    request: SettingsRequest,
) -> IpcResponse {
    match request {
        // ── Governance (roles, moderation, invites) ──────────────
        // ── Profile, pictures and policy (plan E3.1 D4) ──────────
        request @ (SettingsRequest::CommunityUpdateProfile { .. }
        | SettingsRequest::CommunitySetAvatar { .. }
        | SettingsRequest::CommunitySetBanner { .. }
        | SettingsRequest::CommunityAvatarGet { .. }
        | SettingsRequest::CommunityPolicyGet { .. }
        | SettingsRequest::CommunityPolicySet { .. }) => {
            community_profile::route(ctx, state, request).await
        }
        // ── Onboarding (plan E3.1 D3) ────────────────────────────
        request @ (SettingsRequest::OnboardingConfigGet { .. }
        | SettingsRequest::OnboardingConfigSet { .. }
        | SettingsRequest::WelcomeScreenGet { .. }
        | SettingsRequest::WelcomeScreenSet { .. }
        | SettingsRequest::OnboardingSubmit { .. }
        | SettingsRequest::OnboardingComplete { .. }) => {
            onboarding::route(ctx, state, request).await
        }
    }
}

/// `route` for the channel and category requests.
pub(super) async fn route_channels(
    ctx: &DaemonContext,
    state: DaemonState,
    request: ChannelRequest,
) -> IpcResponse {
    match request {
        // ── Channel ──────────────────────────────────────────────
        request @ (ChannelRequest::ChannelList { .. }
        | ChannelRequest::ChannelCreate { .. }
        | ChannelRequest::ChannelDelete { .. }
        | ChannelRequest::ChannelUpdate { .. }
        | ChannelRequest::ChannelRename { .. }
        | ChannelRequest::ChannelMove { .. }
        | ChannelRequest::ChannelReorder { .. }
        | ChannelRequest::ChannelSetTopic { .. }
        | ChannelRequest::ChannelSetSlowmode { .. }
        | ChannelRequest::ChannelSetForumTags { .. }
        | ChannelRequest::ChannelOverwriteSet { .. }
        | ChannelRequest::ChannelOverwriteDelete { .. }
        | ChannelRequest::CategoryCreate { .. }
        | ChannelRequest::CategoryDelete { .. }
        | ChannelRequest::CategoryRename { .. }
        | ChannelRequest::CategoryReorder { .. }) => {
            channel::route_channels(ctx, state, request).await
        }
    }
}

/// `route` for the channel message and channel key requests.
pub(super) async fn route_messages(
    ctx: &DaemonContext,
    state: DaemonState,
    request: ChannelMessageRequest,
) -> IpcResponse {
    match request {
        // ── Channel ──────────────────────────────────────────────
        request @ ChannelMessageRequest::ChannelSend { .. } => {
            channel::route_messages(ctx, state, request).await
        }
        request @ (ChannelMessageRequest::ChannelMessageSend { .. }
        | ChannelMessageRequest::ChannelMessageEdit { .. }
        | ChannelMessageRequest::ChannelMessageDelete { .. }) => {
            channel_message::handle(ctx, state, request).await
        }
        request @ (ChannelMessageRequest::ChannelMessageForward { .. }
        | ChannelMessageRequest::ChannelMessageAdminDelete { .. }
        | ChannelMessageRequest::ChannelMessageBulkDelete { .. }
        | ChannelMessageRequest::ChannelTyping { .. }
        | ChannelMessageRequest::ChannelSetActive { .. }) => {
            channel_message_ops::handle(ctx, state, request).await
        }
        request @ (ChannelMessageRequest::ChannelMessages { .. }
        | ChannelMessageRequest::ChannelMessagesBefore { .. }) => {
            channel_message::handle_read(ctx, state, request).await
        }
        // ── Channel keys ─────────────────────────────────────────
        ChannelMessageRequest::MekList { community } => {
            keys::handle_mek_list(ctx, state, &community)
        }
        ChannelMessageRequest::MekRotate { community, channel } => {
            keys::handle_mek_rotate(ctx, state, &community, &channel)
        }
        ChannelMessageRequest::MekRequest {
            community,
            channel,
            generation,
        } => keys::handle_mek_request(ctx, state, &community, &channel, generation),
    }
}

/// `route` for the read marker requests.
pub(super) async fn route_unread(
    ctx: &DaemonContext,
    state: DaemonState,
    request: UnreadRequest,
) -> IpcResponse {
    match request {
        // ── Unread (plan E3.1 G1) ────────────────────────────────
        request @ (UnreadRequest::ChannelMarkRead { .. } | UnreadRequest::UnreadCounts { .. }) => {
            unread::handle(ctx, state, request).await
        }
    }
}

/// `route` for the audit and notification default requests.
pub(super) async fn route_audit(
    ctx: &DaemonContext,
    state: DaemonState,
    request: AuditRequest,
) -> IpcResponse {
    match request {
        request @ (AuditRequest::AuditLog { .. }
        | AuditRequest::NotificationDefaultGet { .. }
        | AuditRequest::NotificationDefaultSet { .. }) => audit::handle(ctx, state, request).await,
    }
}

/// `route` for the event and game server requests.
pub(super) async fn route_activities(
    ctx: &DaemonContext,
    state: DaemonState,
    request: ActivityRequest,
) -> IpcResponse {
    match request {
        // ── Events (plan E3.1 E4) ────────────────────────────────
        request @ (ActivityRequest::EventList { .. }
        | ActivityRequest::EventAttendees { .. }
        | ActivityRequest::EventCreate { .. }
        | ActivityRequest::EventEdit { .. }
        | ActivityRequest::EventCancel { .. }
        | ActivityRequest::EventDelete { .. }
        | ActivityRequest::EventRsvp { .. }) => events::handle(ctx, state, request).await,
        // ── Game servers (plan E3.1 H1) ──────────────────────────
        request @ (ActivityRequest::GameServerList { .. }
        | ActivityRequest::GameServerAdd { .. }
        | ActivityRequest::GameServerRemove { .. }) => {
            game_servers::handle(ctx, state, request).await
        }
    }
}

/// `route` for the role and moderation requests.
pub(super) async fn route_members(
    ctx: &DaemonContext,
    state: DaemonState,
    request: MemberRequest,
) -> IpcResponse {
    match request {
        request @ (MemberRequest::AutomodRules { .. }
        | MemberRequest::AutomodRuleSet { .. }
        | MemberRequest::AutomodRuleDelete { .. }) => automod::handle(ctx, state, request).await,
        MemberRequest::RoleList { community } => {
            governance::handle_role_list(ctx, state, &community)
        }
        MemberRequest::RoleCreate {
            community,
            name,
            permissions,
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group,
        } => {
            let role = governance::NewRole {
                name,
                color,
                permissions,
                hoist,
                mentionable,
                self_assignable,
                exclusion_group,
            };
            governance::handle_role_create(ctx, state, &community, role).await
        }
        MemberRequest::RoleUpdate {
            community,
            role_id,
            name,
            permissions,
            color,
            position,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group,
        } => {
            let patch = governance::RoleSnapshotPatch {
                name,
                color,
                permissions,
                position,
                hoist,
                mentionable,
                self_assignable,
                exclusion_group,
            };
            governance::handle_role_update(ctx, state, &community, role_id, patch).await
        }
        MemberRequest::RoleDelete { community, role_id } => {
            governance::handle_role_delete(ctx, state, &community, role_id).await
        }
        MemberRequest::RoleAssign {
            community,
            member_pseudonym,
            role_id,
        } => {
            governance::handle_role_grant(ctx, state, &community, &member_pseudonym, role_id, true)
                .await
        }
        MemberRequest::RoleUnassign {
            community,
            member_pseudonym,
            role_id,
        } => {
            governance::handle_role_grant(ctx, state, &community, &member_pseudonym, role_id, false)
                .await
        }
        MemberRequest::RoleSelfAssign { community, role_id } => {
            governance::handle_role_self(ctx, state, &community, role_id, true).await
        }
        MemberRequest::RoleSelfUnassign { community, role_id } => {
            governance::handle_role_self(ctx, state, &community, role_id, false).await
        }
        MemberRequest::Kick {
            community,
            target_pseudonym,
        } => governance::handle_kick(ctx, state, &community, &target_pseudonym),
        MemberRequest::Ban {
            community,
            target_pseudonym,
            reason,
        } => {
            governance::handle_ban(ctx, state, &community, &target_pseudonym, reason.as_deref())
                .await
        }
        MemberRequest::Unban {
            community,
            target_pseudonym,
        } => governance::handle_unban(ctx, state, &community, &target_pseudonym).await,
        MemberRequest::Timeout {
            community,
            target_pseudonym,
            duration_seconds,
            reason,
        } => {
            governance::handle_timeout(
                ctx,
                state,
                &community,
                &target_pseudonym,
                duration_seconds,
                reason.as_deref(),
            )
            .await
        }
        MemberRequest::TimeoutRemove {
            community,
            target_pseudonym,
        } => governance::handle_timeout_remove(ctx, state, &community, &target_pseudonym).await,
        request @ (MemberRequest::MemberBlock { .. }
        | MemberRequest::MemberUnblock { .. }
        | MemberRequest::MemberBlockList { .. }) => member_block::handle(ctx, state, request).await,
        MemberRequest::BanList { community } => {
            governance::handle_ban_list(ctx, state, &community).await
        }
        MemberRequest::ModReportList { community } => {
            governance::handle_mod_report_list(ctx, state, &community)
        }
    }
}

/// `route` for the pin and search requests.
pub(super) async fn route_pins(
    ctx: &DaemonContext,
    state: DaemonState,
    request: PinRequest,
) -> IpcResponse {
    match request {
        request @ (PinRequest::ChannelPins { .. }
        | PinRequest::MessagePin { .. }
        | PinRequest::MessageUnpin { .. }
        | PinRequest::AttachmentPin { .. }
        | PinRequest::MessageSearch { .. }) => pins::handle(ctx, state, request).await,
    }
}

/// `route` for the reaction, thread and poll requests.
pub(super) async fn route_rich(
    ctx: &DaemonContext,
    state: DaemonState,
    request: RichRequest,
) -> IpcResponse {
    match request {
        request @ (RichRequest::ReactionAdd { .. }
        | RichRequest::ReactionRemove { .. }
        | RichRequest::ThreadCreate { .. }
        | RichRequest::ThreadArchive { .. }
        | RichRequest::ThreadUnarchive { .. }
        | RichRequest::ThreadList { .. }
        | RichRequest::ThreadListActive { .. }
        | RichRequest::ThreadMessages { .. }
        | RichRequest::ThreadSend { .. }
        | RichRequest::PollCreate { .. }
        | RichRequest::PollVote { .. }
        | RichRequest::PollClose { .. }
        | RichRequest::PollResults { .. }) => rich_messaging::handle(ctx, state, request).await,
    }
}

/// `route` for the expression requests.
pub(super) async fn route_expressions(
    ctx: &DaemonContext,
    state: DaemonState,
    request: ExpressionRequest,
) -> IpcResponse {
    match request {
        request @ (ExpressionRequest::EmojiUpload { .. }
        | ExpressionRequest::StickerUpload { .. }
        | ExpressionRequest::ExpressionDelete { .. }
        | ExpressionRequest::Expressions { .. }) => expressions::handle(ctx, state, request).await,
    }
}
