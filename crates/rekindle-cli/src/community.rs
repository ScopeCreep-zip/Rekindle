//! Community commands: create, join, leave, list, info.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::CommunityCmd;
use crate::config::schema::Config;
use crate::helpers;
use crate::output::{format, table};
use crate::output::OutputMode;
use crate::transport::DaemonClient;

pub async fn dispatch(cmd: &CommunityCmd, client: &DaemonClient, _cfg: &Config, mode: OutputMode) -> anyhow::Result<()> {
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
        CommunityCmd::Leave { community, .. } => {
            let confirmed = helpers::confirm(&format!("Leave community '{community}'?"))?;
            if !confirmed { return format::print_text("Cancelled."); }
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
                ("Members", value.get("member_count").and_then(serde_json::Value::as_u64).map_or("?".into(), |n| n.to_string())),
                ("Channels", value.get("channels").and_then(|v| v.as_array()).map_or("0".into(), |a| a.len().to_string())),
                ("Our Pseudonym", value.get("our_pseudonym").and_then(|v| v.as_str()).unwrap_or("?").to_string()),
                ("Governance Key", value.get("governance_key").and_then(|v| v.as_str()).unwrap_or("?").to_string()),
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
            let slot = value.get("slot").and_then(serde_json::Value::as_u64).unwrap_or(0);
            format::print_text(&format!("Approved {member} (slot {slot})"))
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
            format::print_text(&format!("Rejected {member}"))
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
                let confirmed = helpers::confirm(&format!("Transfer ownership of '{community}' to {new_owner}?"))?;
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
            format::print_text(&format!("Ownership transferred to {new_owner}"))
        }
        CommunityCmd::Invite(sub) => crate::governance::dispatch_invite(sub, client, mode).await,
    }
}
