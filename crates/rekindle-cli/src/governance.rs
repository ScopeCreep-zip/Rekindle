//! Governance commands under `community`: roles, moderation, invites.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_ipc::protocol::{LifecycleRequest, MemberRequest};

use crate::cli::{InviteCmd, ModerateCmd, RoleCmd};
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

/// A role id: the full 16 bytes as 32 hex characters (plan E3.2 S9).
fn parse_role_id(s: &str) -> anyhow::Result<String> {
    rekindle_types::id::RoleId::from_hex(s)
        .map(rekindle_types::id::RoleId::to_hex)
        .ok_or_else(|| anyhow::anyhow!("invalid role id (32 hex characters): {s}"))
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

/// A member target: the pseudonym (or the name the community shows) the
/// Host resolves; never empty, never a control character.
fn parse_member(s: &str) -> anyhow::Result<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--member cannot be empty");
    }
    if trimmed.chars().any(char::is_control) {
        anyhow::bail!("--member cannot contain control characters");
    }
    Ok(trimmed.to_string())
}

fn members(request: MemberRequest) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Members(request))
}

/// The Host request a `community role` subcommand sends.
pub(crate) fn role_request(cmd: &RoleCmd) -> anyhow::Result<IpcRequest> {
    Ok(members(match cmd {
        RoleCmd::List { community } => MemberRequest::RoleList {
            community: community.clone(),
        },
        RoleCmd::Create {
            community,
            name,
            permissions,
            color,
        } => MemberRequest::RoleCreate {
            community: community.clone(),
            name: helpers::validate_name(name, "Role")?,
            permissions: parse_permissions(permissions)?,
            color: color.as_deref().map(parse_color).transpose()?.unwrap_or(0),
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        },
        RoleCmd::Edit {
            community,
            role_id,
            name,
            permissions,
            color,
        } => {
            if name.is_none() && permissions.is_none() && color.is_none() {
                anyhow::bail!("nothing to change: give --name, --permissions or --color");
            }
            MemberRequest::RoleUpdate {
                community: community.clone(),
                role_id: parse_role_id(role_id)?,
                name: name
                    .as_deref()
                    .map(|n| helpers::validate_name(n, "Role"))
                    .transpose()?,
                permissions: permissions.as_deref().map(parse_permissions).transpose()?,
                color: color.as_deref().map(parse_color).transpose()?,
                position: None,
                hoist: None,
                mentionable: None,
                self_assignable: None,
                exclusion_group: rekindle_types::governance::ExclusionGroupEdit::Unchanged,
            }
        }
        RoleCmd::Delete { community, role_id } => MemberRequest::RoleDelete {
            community: community.clone(),
            role_id: parse_role_id(role_id)?,
        },
        RoleCmd::Assign {
            community,
            member,
            role_id,
        } => MemberRequest::RoleAssign {
            community: community.clone(),
            member_pseudonym: parse_member(member)?,
            role_id: parse_role_id(role_id)?,
        },
        RoleCmd::Unassign {
            community,
            member,
            role_id,
        } => MemberRequest::RoleUnassign {
            community: community.clone(),
            member_pseudonym: parse_member(member)?,
            role_id: parse_role_id(role_id)?,
        },
    }))
}

/// `role create`'s answer (the new role's id, hex) as `{"roleId": …}`.
pub(crate) fn role_created(value: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    match value {
        serde_json::Value::String(id) => Ok(serde_json::json!({ "roleId": id })),
        other => anyhow::bail!("role create answered no role id: {other}"),
    }
}

pub async fn dispatch_role(
    cmd: &RoleCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let value = helpers::send(client, role_request(cmd)?).await?;
    let value = if matches!(cmd, RoleCmd::Create { .. }) {
        role_created(value)?
    } else {
        value
    };
    format::print_structured(&value, mode)
}

/// The Host request a moderation subcommand (`community kick`, `ban`,
/// `unban`, `timeout`, `bans`) sends.
pub(crate) fn moderate_request(cmd: &ModerateCmd) -> anyhow::Result<IpcRequest> {
    Ok(members(match cmd {
        ModerateCmd::Kick {
            community,
            member,
            reason,
        } => MemberRequest::Kick {
            community: community.clone(),
            target_pseudonym: parse_member(member)?,
            reason: reason.clone(),
        },
        ModerateCmd::Ban {
            community,
            member,
            reason,
        } => MemberRequest::Ban {
            community: community.clone(),
            target_pseudonym: parse_member(member)?,
            reason: reason.clone(),
        },
        ModerateCmd::Unban { community, member } => MemberRequest::Unban {
            community: community.clone(),
            target_pseudonym: parse_member(member)?,
        },
        ModerateCmd::Timeout {
            community,
            member,
            duration,
            reason,
        } => MemberRequest::Timeout {
            community: community.clone(),
            target_pseudonym: parse_member(member)?,
            duration_seconds: parse_duration_secs(duration)?,
            reason: reason.clone(),
        },
        ModerateCmd::Bans { community } => MemberRequest::BanList {
            community: community.clone(),
        },
    }))
}

pub async fn dispatch_moderate(
    cmd: &ModerateCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    helpers::send_and_print(client, moderate_request(cmd)?, mode).await
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
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::InviteCreate {
                        community: community.clone(),
                        max_uses: *max_uses,
                        expires_seconds: exp_secs,
                    },
                )),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        InviteCmd::List { community } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::InviteList {
                    community: community.clone(),
                })),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        InviteCmd::Revoke { invite_code } => {
            // InviteRevoke needs the community — we send empty and let daemon resolve
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::InviteRevoke {
                        community: String::new(),
                        invite_code: invite_code.clone(),
                    },
                )),
            )
            .await?;
            format::print_structured(&value, mode)
        }
    }
}

#[cfg(test)]
#[path = "governance_tests.rs"]
mod tests;
