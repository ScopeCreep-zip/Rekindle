//! Governance commands: roles, moderation, invites.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};

use crate::cli::{InviteCmd, ModerateCmd, RoleCmd};
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

fn parse_u32(s: &str) -> anyhow::Result<u32> {
    s.parse()
        .map_err(|_| anyhow::anyhow!("invalid number: {s}"))
}

fn parse_permissions(s: &str) -> anyhow::Result<u64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|_| anyhow::anyhow!("invalid hex permissions: {s}"))
    } else {
        s.parse()
            .map_err(|_| anyhow::anyhow!("invalid permissions: {s}"))
    }
}

fn parse_duration_secs(s: &str) -> anyhow::Result<u64> {
    if let Some(n) = s.strip_suffix('s') {
        return n.parse().map_err(Into::into);
    }
    if let Some(n) = s.strip_suffix('m') {
        return Ok(n.parse::<u64>()? * 60);
    }
    if let Some(n) = s.strip_suffix('h') {
        return Ok(n.parse::<u64>()? * 3600);
    }
    if let Some(n) = s.strip_suffix('d') {
        return Ok(n.parse::<u64>()? * 86400);
    }
    if let Some(n) = s.strip_suffix('w') {
        return Ok(n.parse::<u64>()? * 604_800);
    }
    s.parse()
        .map_err(|_| anyhow::anyhow!("invalid duration: {s} (use 30s, 5m, 1h, 24h, 7d)"))
}

fn parse_color(s: &str) -> anyhow::Result<u32> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    u32::from_str_radix(hex, 16).map_err(|_| anyhow::anyhow!("invalid color hex: {s}"))
}

pub async fn dispatch_role(
    cmd: &RoleCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        RoleCmd::List { community } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::RoleList {
                    community: community.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Create {
            community,
            name,
            permissions,
            color,
        } => {
            let perms = permissions
                .as_deref()
                .map(parse_permissions)
                .transpose()?
                .unwrap_or(0);
            let col = color.as_deref().map(parse_color).transpose()?.unwrap_or(0);
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::RoleCreate {
                    community: community.clone(),
                    name: name.clone(),
                    permissions: perms,
                    color: col,
                    hoist: false,
                    mentionable: false,
                    self_assignable: false,
                    exclusion_group: None,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Update {
            community,
            role_id,
            name,
            permissions,
            color,
        } => {
            let rid = parse_u32(role_id)?;
            let perms = permissions.as_deref().map(parse_permissions).transpose()?;
            let col = color.as_deref().map(parse_color).transpose()?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::RoleUpdate {
                    community: community.clone(),
                    role_id: rid,
                    name: name.clone(),
                    permissions: perms,
                    color: col,
                    position: None,
                    hoist: None,
                    mentionable: None,
                    self_assignable: None,
                    exclusion_group: rekindle_types::governance::ExclusionGroupEdit::Unchanged,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Delete { community, role_id } => {
            let rid = parse_u32(role_id)?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::RoleDelete {
                    community: community.clone(),
                    role_id: rid,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Assign {
            community,
            member,
            role_id,
        } => {
            let rid = parse_u32(role_id)?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::RoleAssign {
                    community: community.clone(),
                    member_pseudonym: member.clone(),
                    role_id: rid,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Unassign {
            community,
            member,
            role_id,
        } => {
            let rid = parse_u32(role_id)?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::RoleUnassign {
                    community: community.clone(),
                    member_pseudonym: member.clone(),
                    role_id: rid,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
    }
}

pub async fn dispatch_moderate(
    cmd: &ModerateCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ModerateCmd::Kick {
            community, member, ..
        } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Kick {
                    community: community.clone(),
                    target_pseudonym: member.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ModerateCmd::Ban {
            community,
            member,
            reason,
        } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Ban {
                    community: community.clone(),
                    target_pseudonym: member.clone(),
                    reason: reason.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ModerateCmd::Unban { community, member } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Unban {
                    community: community.clone(),
                    target_pseudonym: member.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ModerateCmd::Timeout {
            community,
            member,
            duration,
            reason,
        } => {
            let secs = parse_duration_secs(duration)?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Timeout {
                    community: community.clone(),
                    target_pseudonym: member.clone(),
                    duration_seconds: secs,
                    reason: reason.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        ModerateCmd::Bans { community } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::BanList {
                    community: community.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
    }
}

pub async fn dispatch_invite(
    cmd: &InviteCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        InviteCmd::Create {
            community,
            max_uses,
            expires,
        } => {
            let exp_secs = expires.as_deref().map(parse_duration_secs).transpose()?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::InviteCreate {
                    community: community.clone(),
                    max_uses: max_uses.unwrap_or(0),
                    expires_seconds: exp_secs,
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        InviteCmd::List { community } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::InviteList {
                    community: community.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        InviteCmd::Revoke { invite_code } => {
            // InviteRevoke needs the community — we send empty and let daemon resolve
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::InviteRevoke {
                    community: String::new(),
                    invite_code: invite_code.clone(),
                }),
            )
            .await?;
            format::print_structured(&value, mode)
        }
    }
}
