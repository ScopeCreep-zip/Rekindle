//! Community lifecycle commands — create, join, leave, list, info.
//! Role management and moderation commands.

mod create;
mod info;
mod invite;
mod join;
mod leave;
mod list;
mod moderation;
mod roles;

use crate::cli::{CommunityCmd, InviteCmd, ModerateCmd, RoleCmd};
use crate::config::schema::Config;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Dispatch `rekindle community <subcommand>`.
pub async fn dispatch(
    cmd: &CommunityCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    _cfg: &Config,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        CommunityCmd::Create {
            name,
            description,
            icon: _,
        } => create::cmd_create(handle, session, name, description.as_deref(), mode).await,
        CommunityCmd::Join {
            invite,
            display_name,
        } => join::cmd_join(handle, session, invite, display_name.as_deref(), mode).await,
        CommunityCmd::Leave { community, yes } => {
            leave::cmd_leave(handle, session, community, *yes, mode).await
        }
        CommunityCmd::List { format: fmt } => {
            let effective_mode = fmt
                .as_deref()
                .map_or(mode, |f| match f {
                    "json" => OutputMode::Json,
                    "jsonl" => OutputMode::Jsonl,
                    _ => mode,
                });
            list::cmd_list(handle, session, effective_mode).await
        }
        CommunityCmd::Info { community, verbose } => {
            info::cmd_info(handle, session, community, *verbose, mode).await
        }
        CommunityCmd::Invite(inv_cmd) => dispatch_invite(inv_cmd, handle, session, mode).await,
    }
}

/// Dispatch `rekindle community invite <subcommand>`.
async fn dispatch_invite(
    cmd: &InviteCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        InviteCmd::Create {
            community,
            max_uses,
            expires,
        } => {
            invite::cmd_invite_create(
                handle,
                session,
                community,
                *max_uses,
                expires.as_deref(),
                mode,
            )
            .await
        }
        InviteCmd::List { community } => {
            invite::cmd_invite_list(handle, session, community, mode).await
        }
        InviteCmd::Revoke { invite_code } => {
            invite::cmd_invite_revoke(handle, session, invite_code, mode).await
        }
    }
}

/// Dispatch `rekindle role <subcommand>`.
pub async fn dispatch_role(
    cmd: &RoleCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        RoleCmd::List { community } => {
            roles::cmd_role_list(handle, session, community, mode).await
        }
        RoleCmd::Create {
            community,
            name,
            permissions,
            color,
            position,
        } => {
            roles::cmd_role_create(
                handle,
                session,
                community,
                name,
                permissions.as_deref(),
                color.as_deref(),
                *position,
                mode,
            )
            .await
        }
        RoleCmd::Update {
            community,
            role_id,
            name,
            permissions,
            color,
        } => {
            roles::cmd_role_update(
                handle,
                session,
                community,
                role_id,
                name.as_deref(),
                permissions.as_deref(),
                color.as_deref(),
                mode,
            )
            .await
        }
        RoleCmd::Delete {
            community,
            role_id,
            yes,
        } => {
            roles::cmd_role_delete(handle, session, community, role_id, *yes, mode).await
        }
        RoleCmd::Assign {
            community,
            member,
            role_id,
        } => roles::cmd_role_assign(handle, session, community, member, role_id, mode).await,
        RoleCmd::Unassign {
            community,
            member,
            role_id,
        } => roles::cmd_role_unassign(handle, session, community, member, role_id, mode).await,
    }
}

/// Dispatch `rekindle moderate <subcommand>`.
pub async fn dispatch_moderate(
    cmd: &ModerateCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ModerateCmd::Kick {
            community,
            member,
            reason,
        } => {
            moderation::cmd_kick(handle, session, community, member, reason.as_deref(), mode)
        }
        ModerateCmd::Ban {
            community,
            member,
            reason,
        } => {
            moderation::cmd_ban(handle, session, community, member, reason.as_deref(), mode).await
        }
        ModerateCmd::Unban {
            community,
            member,
        } => moderation::cmd_unban(handle, session, community, member, mode).await,
        ModerateCmd::Timeout {
            community,
            member,
            duration,
            reason,
        } => {
            moderation::cmd_timeout(
                handle,
                session,
                community,
                member,
                duration,
                reason.as_deref(),
                mode,
            )
        }
        ModerateCmd::Bans { community } => {
            moderation::cmd_bans(handle, session, community, mode).await
        }
    }
}
