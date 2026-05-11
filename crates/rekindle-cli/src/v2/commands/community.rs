//! Community commands: create, join, leave, list, info, approve, reject, pending, transfer.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::cli::CommunityCmd;
use crate::v2::helpers;
use crate::v2::output::{format, table};
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

pub async fn dispatch(cmd: &CommunityCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        CommunityCmd::Create { name, description, .. } => {
            let validated_name = helpers::validate_name(name, "Community")?;
            let value = client.request_ok(IpcRequest::CommunityCreate {
                name: validated_name,
                description: description.clone().unwrap_or_default(),
            }).await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::Join { invite, .. } => {
            let value = client.request_ok(IpcRequest::CommunityJoin {
                invite: invite.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::Leave { community, yes } => {
            if !yes {
                let confirmed = helpers::confirm(&format!("Leave community '{community}'?"))?;
                if !confirmed { return format::print_text("Cancelled."); }
            }
            let value = client.request_ok(IpcRequest::CommunityLeave {
                governance_key: community.clone(),
            }).await?;
            helpers::audit_log("leave_community", community, "ok");
            format::print_structured(&value, mode)
        }
        CommunityCmd::List { .. } => {
            let value = client.request_ok(IpcRequest::CommunityList).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let rows = value.as_array().map(|arr| {
                arr.iter().map(|c| vec![
                    c.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                    c.get("member_count").and_then(serde_json::Value::as_u64).map_or("?".into(), |n| n.to_string()),
                    c.get("channel_count").and_then(serde_json::Value::as_u64).map_or("?".into(), |n| n.to_string()),
                    helpers::abbreviate_key(c.get("governance_key").and_then(|v| v.as_str()).unwrap_or("?")),
                ]).collect::<Vec<_>>()
            }).unwrap_or_default();
            table::print_table(&["Name", "Members", "Channels", "Key"], &rows, mode)
        }
        CommunityCmd::Info { community, .. } => {
            let value = client.request_ok(IpcRequest::CommunityInfo {
                governance_key: community.clone(),
            }).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let pairs = vec![
                ("Name", value.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string()),
                ("Description", value.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string()),
                ("Owner", helpers::abbreviate_key(value.get("owner_pseudonym").and_then(|v| v.as_str()).unwrap_or("?"))),
                ("Join Policy", value.get("join_policy").and_then(|v| v.as_str()).unwrap_or("?").to_string()),
                ("Channels", value.get("channel_count").and_then(serde_json::Value::as_u64).map_or("?".into(), |n| n.to_string())),
                ("Roles", value.get("role_count").and_then(serde_json::Value::as_u64).map_or("?".into(), |n| n.to_string())),
                ("Our Pseudonym", helpers::abbreviate_key(value.get("our_pseudonym").and_then(|v| v.as_str()).unwrap_or("?"))),
                ("Operator", value.get("is_operator").and_then(serde_json::Value::as_bool).map_or("no".into(), |b| if b { "yes".into() } else { "no".into() })),
                ("Locked Down", value.get("locked_down").and_then(serde_json::Value::as_bool).map_or("no".into(), |b| if b { "yes".into() } else { "no".into() })),
            ];
            table::print_kv_table(&pairs, mode)
        }
        CommunityCmd::Approve { community, member } => {
            let value = client.request_ok(IpcRequest::CommunityApprove {
                governance_key: community.clone(),
                member_pseudonym: member.clone(),
            }).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!("Approved {}", helpers::abbreviate_key(member)))
        }
        CommunityCmd::Reject { community, member, reason } => {
            let value = client.request_ok(IpcRequest::CommunityReject {
                governance_key: community.clone(),
                member_pseudonym: member.clone(),
                reason: reason.clone().unwrap_or_default(),
            }).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!("Rejected {}", helpers::abbreviate_key(member)))
        }
        CommunityCmd::Pending { community } => {
            let value = client.request_ok(IpcRequest::CommunityPendingMembers {
                governance_key: community.clone(),
            }).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let rows = value.as_array().map(|arr| {
                arr.iter().map(|p| vec![
                    helpers::abbreviate_key(p.get("requester_pseudonym_hex").and_then(|v| v.as_str()).unwrap_or("?")),
                    p.get("display_name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                    p.get("status").and_then(|v| v.as_str()).unwrap_or("pending").to_string(),
                ]).collect::<Vec<_>>()
            }).unwrap_or_default();
            table::print_table(&["Pseudonym", "Name", "Status"], &rows, mode)
        }
        CommunityCmd::Transfer { community, new_owner, yes } => {
            if !yes {
                let confirmed = helpers::confirm(&format!("Transfer ownership of '{community}' to {}?", helpers::abbreviate_key(new_owner)))?;
                if !confirmed { return format::print_text("Cancelled."); }
            }
            let value = client.request_ok(IpcRequest::CommunityTransferOwnership {
                governance_key: community.clone(),
                new_owner_pseudonym: new_owner.clone(),
            }).await?;
            helpers::audit_log("transfer_ownership", community, "ok");
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!("Ownership transferred to {}", helpers::abbreviate_key(new_owner)))
        }
        CommunityCmd::Invite(sub) => crate::v2::commands::governance::dispatch_invite(sub, client, mode).await,
    }
}
