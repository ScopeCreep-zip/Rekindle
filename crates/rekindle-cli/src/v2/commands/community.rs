//! Community commands: create, join, leave, list, info, approve, reject, pending, transfer.

use crate::v2::prelude::{ChatRequest, DaemonRequest};

use crate::v2::cli::CommunityCmd;
use crate::v2::helpers;
use crate::v2::output::{format, table};
use crate::v2::output::OutputMode;
use crate::v2::prelude::DaemonClient;

pub async fn dispatch(cmd: &CommunityCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        CommunityCmd::Create { name, description, .. } => {
            let validated_name = helpers::validate_name(name, "Community")?;
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityCreate {
                name: validated_name,
                description: description.clone().unwrap_or_default(),
            })).await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::Join { invite, .. } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityJoin {
                invite: invite.clone(),
            })).await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::Leave { community, yes } => {
            if !yes {
                let confirmed = helpers::confirm(&format!("Leave community '{community}'?"))?;
                if !confirmed { return format::print_text("Cancelled."); }
            }
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityLeave {
                governance_key: community.clone(),
            })).await?;
            helpers::audit_log("leave_community", community, "ok");
            format::print_structured(&value, mode)
        }
        CommunityCmd::List { .. } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityList)).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let rows = value.as_array().map(|arr| {
                arr.iter().map(|c| vec![
                    c.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                    if c.get("is_operator").and_then(|v| v.as_bool()).unwrap_or(false) { "yes".into() } else { "no".into() },
                    helpers::abbreviate_key(c.get("pseudonym").and_then(|v| v.as_str()).unwrap_or("?")),
                    helpers::abbreviate_key(c.get("governance_key").and_then(|v| v.as_str()).unwrap_or("?")),
                ]).collect::<Vec<_>>()
            }).unwrap_or_default();
            table::print_table(&["Name", "Operator", "Pseudonym", "Key"], &rows, mode)
        }
        CommunityCmd::Info { community, detailed } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityInfo {
                governance_key: community.clone(),
            })).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let pairs = vec![
                ("Name", value.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string()),
                ("Description", value.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string()),
                ("Owner", helpers::abbreviate_key(value.get("owner_pseudonym").and_then(|v| v.as_str()).unwrap_or("?"))),
                ("Join Policy", value.get("join_policy").and_then(|v| v.as_str()).unwrap_or("?").to_string()),
                ("Members", value.get("member_count").and_then(serde_json::Value::as_u64).map_or("?".into(), |n| n.to_string())),
                ("Channels", value.get("channel_count").and_then(serde_json::Value::as_u64).map_or("?".into(), |n| n.to_string())),
                ("Roles", value.get("role_count").and_then(serde_json::Value::as_u64).map_or("?".into(), |n| n.to_string())),
                ("Our Pseudonym", helpers::abbreviate_key(value.get("our_pseudonym").and_then(|v| v.as_str()).unwrap_or("?"))),
                ("Operator", value.get("is_operator").and_then(serde_json::Value::as_bool).map_or("no".into(), |b| if b { "yes".into() } else { "no".into() })),
                ("Locked Down", value.get("locked_down").and_then(serde_json::Value::as_bool).map_or("no".into(), |b| if b { "yes".into() } else { "no".into() })),
            ];
            table::print_kv_table(&pairs, mode)?;

            if *detailed {
                use std::io::Write;
                let mut out = std::io::stdout().lock();

                // Channels with unread counts
                if let Some(channels) = value.get("channels").and_then(|v| v.as_array()) {
                    let unreads = value.get("channel_unreads").and_then(|v| v.as_object());
                    writeln!(out, "\n  Channels ({}):", channels.len())?;
                    for ch in channels {
                        let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let kind = ch.get("kind").and_then(|v| v.as_str()).unwrap_or("text");
                        let topic = ch.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                        let id = ch.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let unread = unreads
                            .and_then(|u| u.get(id))
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0);
                        let unread_str = if unread > 0 { format!("  ({unread} unread)") } else { String::new() };
                        let topic_str = if topic.is_empty() { String::new() } else { format!("  — {topic}") };
                        writeln!(out, "    #{name:<20} {kind}{topic_str}{unread_str}")?;
                    }
                }

                // Members with presence and roles
                if let Some(members) = value.get("members").and_then(|v| v.as_array()) {
                    writeln!(out, "\n  Members ({}):", members.len())?;
                    for m in members {
                        let name = m.get("displayName").or_else(|| m.get("display_name"))
                            .and_then(|v| v.as_str()).unwrap_or("?");
                        let pseudo = m.get("pseudonymKey").or_else(|| m.get("pseudonym_key"))
                            .and_then(|v| v.as_str()).unwrap_or("?");
                        let status = m.get("status").and_then(|v| v.as_str()).unwrap_or("offline");
                        let role = m.get("roleName").or_else(|| m.get("role_name"))
                            .and_then(|v| v.as_str());
                        let (glyph, label) = match status {
                            "online" => ("●", "[ONLINE]"),
                            "away" => ("◐", "[AWAY]"),
                            "busy" => ("●", "[BUSY]"),
                            _ => ("○", "[OFFLINE]"),
                        };
                        let role_str = role.map_or(String::new(), |r| format!("  [{r}]"));
                        let key_short = helpers::abbreviate_key(pseudo);
                        writeln!(out, "    {glyph} {label} {name:<16}{role_str}  {key_short}")?;
                    }
                }

                // Roles with permissions
                if let Some(roles) = value.get("roles").and_then(|v| v.as_array()) {
                    writeln!(out, "\n  Roles ({}):", roles.len())?;
                    for r in roles {
                        let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let id = r.get("id").and_then(serde_json::Value::as_u64).unwrap_or(0);
                        let perms = r.get("permissions").and_then(serde_json::Value::as_u64).unwrap_or(0);
                        let pos = r.get("position").and_then(serde_json::Value::as_i64).unwrap_or(0);
                        writeln!(out, "    {name}  (id: {id}, pos: {pos}, perms: 0x{perms:X})")?;
                    }
                }
            }
            Ok(())
        }
        CommunityCmd::Approve { community, member } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityApprove {
                governance_key: community.clone(),
                member_pseudonym: member.clone(),
            })).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!("Approved {}", helpers::abbreviate_key(member)))
        }
        CommunityCmd::Reject { community, member, reason } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityReject {
                governance_key: community.clone(),
                member_pseudonym: member.clone(),
                reason: reason.clone().unwrap_or_default(),
            })).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!("Rejected {}", helpers::abbreviate_key(member)))
        }
        CommunityCmd::Pending { community } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityPendingMembers {
                governance_key: community.clone(),
            })).await?;
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
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityTransferOwnership {
                governance_key: community.clone(),
                new_owner_pseudonym: new_owner.clone(),
            })).await?;
            helpers::audit_log("transfer_ownership", community, "ok");
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!("Ownership transferred to {}", helpers::abbreviate_key(new_owner)))
        }
        CommunityCmd::Invite(sub) => crate::v2::commands::governance::dispatch_invite(sub, client, mode).await,
    }
}
