//! Governance commands: roles, moderation, invites.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::cli::{RoleCmd, ModerateCmd, InviteCmd};
use crate::v2::helpers;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

pub async fn dispatch_role(cmd: &RoleCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        RoleCmd::List { community } => {
            let value = client.request_ok(IpcRequest::RoleList { community: community.clone() }).await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Create { community, name, permissions, color, position } => {
            let perms = permissions.as_deref().map(helpers::parse_permissions).transpose()?.unwrap_or(0);
            let col = color.as_deref().map(helpers::parse_color).transpose()?.unwrap_or(0);
            #[allow(clippy::cast_possible_wrap)]
            let pos = position.map_or(0, |p| p as i32);
            let value = client.request_ok(IpcRequest::RoleCreate {
                community: community.clone(),
                name: name.clone(),
                permissions: perms,
                color: col,
                position: pos,
            }).await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Update { community, role_id, name, permissions, color } => {
            let rid = helpers::parse_u32(role_id)?;
            let perms = permissions.as_deref().map(helpers::parse_permissions).transpose()?;
            let col = color.as_deref().map(helpers::parse_color).transpose()?;
            let value = client.request_ok(IpcRequest::RoleUpdate {
                community: community.clone(),
                role_id: rid,
                name: name.clone(),
                permissions: perms,
                color: col,
            }).await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Delete { community, role_id, yes } => {
            if !yes {
                let confirmed = helpers::confirm(&format!("Delete role {role_id}?"))?;
                if !confirmed { return format::print_text("Cancelled."); }
            }
            let rid = helpers::parse_u32(role_id)?;
            let value = client.request_ok(IpcRequest::RoleDelete {
                community: community.clone(),
                role_id: rid,
            }).await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Assign { community, member, role_id } => {
            let rid = helpers::parse_u32(role_id)?;
            let value = client.request_ok(IpcRequest::RoleAssign {
                community: community.clone(),
                member_pseudonym: member.clone(),
                role_id: rid,
            }).await?;
            format::print_structured(&value, mode)
        }
        RoleCmd::Unassign { community, member, role_id } => {
            let rid = helpers::parse_u32(role_id)?;
            let value = client.request_ok(IpcRequest::RoleUnassign {
                community: community.clone(),
                member_pseudonym: member.clone(),
                role_id: rid,
            }).await?;
            format::print_structured(&value, mode)
        }
    }
}

pub async fn dispatch_moderate(cmd: &ModerateCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        ModerateCmd::Kick { community, member, .. } => {
            let value = client.request_ok(IpcRequest::Kick {
                community: community.clone(),
                target_pseudonym: member.clone(),
            }).await?;
            helpers::audit_log("kick", member, "ok");
            format::print_structured(&value, mode)
        }
        ModerateCmd::Ban { community, member, reason } => {
            let value = client.request_ok(IpcRequest::Ban {
                community: community.clone(),
                target_pseudonym: member.clone(),
                reason: reason.clone(),
            }).await?;
            helpers::audit_log("ban", member, "ok");
            format::print_structured(&value, mode)
        }
        ModerateCmd::Unban { community, member } => {
            let value = client.request_ok(IpcRequest::Unban {
                community: community.clone(),
                target_pseudonym: member.clone(),
            }).await?;
            helpers::audit_log("unban", member, "ok");
            format::print_structured(&value, mode)
        }
        ModerateCmd::Timeout { community, member, duration, reason } => {
            let secs = helpers::parse_duration_secs(duration)?;
            let value = client.request_ok(IpcRequest::Timeout {
                community: community.clone(),
                target_pseudonym: member.clone(),
                duration_seconds: secs,
                reason: reason.clone(),
            }).await?;
            helpers::audit_log("timeout", member, "ok");
            format::print_structured(&value, mode)
        }
        ModerateCmd::Bans { community } => {
            let value = client.request_ok(IpcRequest::BanList { community: community.clone() }).await?;
            format::print_structured(&value, mode)
        }
    }
}

pub async fn dispatch_invite(cmd: &InviteCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        InviteCmd::Create { community, max_uses, expires } => {
            let exp_secs = expires.as_deref().map(helpers::parse_duration_secs).transpose()?;
            let value = client.request_ok(IpcRequest::InviteCreate {
                community: community.clone(),
                max_uses: max_uses.unwrap_or(0),
                expires_seconds: exp_secs,
            }).await?;
            format::print_structured(&value, mode)
        }
        InviteCmd::List { community } => {
            let value = client.request_ok(IpcRequest::InviteList { community: community.clone() }).await?;
            format::print_structured(&value, mode)
        }
        InviteCmd::Revoke { community, invite_code } => {
            let value = client.request_ok(IpcRequest::InviteRevoke {
                community: community.clone(),
                invite_code: invite_code.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
    }
}
