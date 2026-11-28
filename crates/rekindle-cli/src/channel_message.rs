//! Channel message edits, deletes and moderator removals, and the history
//! as the GUI shows it (plan E3.3 slice 9).
//!
//! Each command sends the same Host request as the GUI
//! (`ChannelMessageEdit`, `ChannelMessageDelete`,
//! `ChannelMessageAdminDelete`); the Host's intent gate, E1–E3 rules and
//! M1–M4 judgement apply unchanged, and its refusals come back as errors.
//! The CLI only renders: "edited", "deleted for members" (E4, never
//! "erased"), "Removed by <name>" (M4).

use std::collections::HashMap;

use rekindle_client::fmt;
use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::{ChannelMessageRequest, LifecycleRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::channel_message::{ChannelMessageView, PostAdmission};
use rekindle_types::dto::community::CommunityMemberView;

use crate::cli::ChannelCmd;
use crate::community_ref;
use crate::helpers;
use crate::output::{format, OutputMode};

/// A channel as the Host names it: the community's anchor key, the
/// channel's id, and our pseudonym in the community.
pub struct ChannelTarget {
    pub anchor: String,
    pub channel_id: String,
    pub our_pseudonym: String,
}

/// Resolve `community` and its channel `channel`; the user may have given
/// either names or keys.
pub async fn target(
    client: &DaemonClient,
    community: &str,
    channel: &str,
) -> anyhow::Result<ChannelTarget> {
    let detail = community_ref::detail(client, community).await?;
    let channel_id = community_ref::channel_id(&detail, channel)?.to_owned();
    Ok(ChannelTarget {
        anchor: detail.anchor,
        channel_id,
        our_pseudonym: detail.our_pseudonym,
    })
}

/// Each member's name in the community `anchor`, by pseudonym.
pub async fn member_names(
    client: &DaemonClient,
    anchor: &str,
) -> anyhow::Result<HashMap<String, String>> {
    let members: Vec<CommunityMemberView> = serde_json::from_value(
        helpers::send(
            client,
            IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityMembers {
                    anchor: anchor.to_owned(),
                },
            )),
        )
        .await?,
    )?;
    Ok(members
        .into_iter()
        .map(|m| (m.pseudonym_key, m.display_name))
        .collect())
}

/// What a change command did, as it is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageChangeOutcome {
    Edited,
    DeletedForMembers,
    /// Removed by the moderator with this pseudonym.
    Removed {
        by: String,
    },
}

/// The Host request for a change command on `target`, or `None` for any
/// other channel command.
pub fn change_request(
    cmd: &ChannelCmd,
    target: &ChannelTarget,
) -> Option<(IpcRequest, MessageChangeOutcome)> {
    let channel = target.channel_id.clone();
    let (request, outcome) = match cmd {
        ChannelCmd::EditMessage {
            msg_id, message, ..
        } => (
            ChannelMessageRequest::ChannelMessageEdit {
                channel,
                message_id: msg_id.clone(),
                body: message.clone(),
            },
            MessageChangeOutcome::Edited,
        ),
        ChannelCmd::DeleteMessage { msg_id, .. } => (
            ChannelMessageRequest::ChannelMessageDelete {
                channel,
                message_id: msg_id.clone(),
            },
            MessageChangeOutcome::DeletedForMembers,
        ),
        ChannelCmd::RemoveMessage { msg_id, reason, .. } => (
            ChannelMessageRequest::ChannelMessageAdminDelete {
                community: target.anchor.clone(),
                channel,
                message_id: msg_id.clone(),
                reason: reason.clone(),
            },
            MessageChangeOutcome::Removed {
                by: target.our_pseudonym.clone(),
            },
        ),
        _ => return None,
    };
    Some((
        IpcRequest::Community(CommunityRequest::Messages(request)),
        outcome,
    ))
}

/// The line a change command prints once the Host ran it.
pub fn outcome_line(
    message_id: &str,
    outcome: &MessageChangeOutcome,
    names: &HashMap<String, String>,
) -> String {
    let id = fmt::abbreviate_key(message_id);
    match outcome {
        MessageChangeOutcome::Edited => format!("Message {id} edited"),
        MessageChangeOutcome::DeletedForMembers => {
            format!("Message {id} {}", fmt::DELETED_FOR_MEMBERS)
        }
        MessageChangeOutcome::Removed { by } => {
            format!(
                "Message {id}: {}",
                fmt::removed_by(&fmt::member_name(names, by))
            )
        }
    }
}

/// `edit-message`, `delete-message`, `remove-message`: send the request
/// (the Host reviews it if its gate asks) and print what it did.
pub async fn dispatch_change(
    cmd: &ChannelCmd,
    community: &str,
    channel: &str,
    msg_id: &str,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let target = target(client, community, channel).await?;
    let (request, outcome) = change_request(cmd, &target)
        .ok_or_else(|| anyhow::anyhow!("not a channel message change"))?;
    helpers::send(client, request).await?;
    if mode.is_structured() {
        let (state, by) = match &outcome {
            MessageChangeOutcome::Edited => ("edited", None),
            MessageChangeOutcome::DeletedForMembers => ("deletedForMembers", None),
            MessageChangeOutcome::Removed { by } => ("removed", Some(by.clone())),
        };
        return format::print_structured(
            &serde_json::json!({ "messageId": msg_id, "state": state, "removedBy": by }),
            mode,
        );
    }
    let names = match outcome {
        MessageChangeOutcome::Removed { .. } => member_names(client, &target.anchor).await?,
        _ => HashMap::new(),
    };
    format::print_text(&outcome_line(msg_id, &outcome, &names))
}

/// One history message after its time, as the GUI's list shows it: the
/// sender's name, then the body with "(edited)", or "Removed by <name>" in
/// place of a moderator-removed body, or the gap line for a gap row.
pub fn history_text(view: &ChannelMessageView, names: &HashMap<String, String>) -> String {
    let sender = fmt::member_name(names, &view.sender_id);
    if view.history_gap_through.is_some() {
        return format!("── {sender}: some earlier messages could not be delivered ──");
    }
    let body = if let Some(admission) = &view.admission {
        match admission {
            PostAdmission::Pending { text }
            | PostAdmission::Refused {
                reason_text: text, ..
            } => text.clone(),
        }
    } else if let Some(by) = &view.removed_by {
        fmt::removed_by(&fmt::member_name(names, by))
    } else if view.decryption_failed {
        "(encrypted — key not yet received)".to_owned()
    } else if view.automod_blurred {
        fmt::AUTOMOD_BLURRED.to_owned()
    } else {
        let body = rekindle_utils::text::sanitize_for_display(&view.body);
        match view.edited_at {
            Some(_) => format!("{body} {}", fmt::EDITED),
            None => body,
        }
    };
    format!("{sender}: {body}")
}

/// `channel history` in text mode: one line per message, oldest first.
pub fn print_history(
    views: &[ChannelMessageView],
    names: &HashMap<String, String>,
) -> anyhow::Result<()> {
    for view in views {
        let time = fmt::format_time_short(u64::try_from(view.timestamp).unwrap_or(0));
        format::print_text(&format!("[{time}] {}", history_text(view, names)))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "channel_message_tests.rs"]
mod tests;
