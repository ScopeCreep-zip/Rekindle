//! Group-DM requests from the TUI (plan E2.6): the list of groups and
//! invites, a group's stored messages, and accept, decline and leave. The
//! host reviews what needs it (`report_failure` shows the review).

use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::group_dm::GroupDmView;

use super::super::action::{Action, CommandResult, ToastLevel};
use super::commands::report_failure;
use super::App;

/// How many of a group's newest messages the view loads.
const PAGE: u32 = 100;

fn parse_failed(
    tx: &tokio::sync::mpsc::UnboundedSender<Action>,
    context: &str,
    e: &serde_json::Error,
) {
    let _ = tx.send(Action::CommandFailed {
        context: context.into(),
        error: e.to_string(),
    });
}

impl App {
    /// Our groups and the invites we have not answered.
    pub(crate) fn load_group_dms(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::GroupDmList).await {
                Ok(value) => match serde_json::from_value::<Vec<GroupDmView>>(value) {
                    Ok(groups) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::GroupDmsLoaded { groups },
                        )));
                    }
                    Err(e) => parse_failed(&tx, "group list parse", &e),
                },
                Err(e) => report_failure(&tx, "group list", e),
            }
        });
    }

    /// The newest messages of `group_id`.
    pub(crate) fn load_group_messages(&self, group_id: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let request = IpcRequest::GroupDmMessages {
                group_id: group_id.clone(),
                limit: PAGE,
                before: None,
            };
            match client.request_ok(request).await {
                Ok(value) => match serde_json::from_value::<Vec<DmMessage>>(value) {
                    Ok(messages) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::GroupMessagesLoaded { group_id, messages },
                        )));
                    }
                    Err(e) => parse_failed(&tx, "group messages parse", &e),
                },
                Err(e) => report_failure(&tx, "group messages", e),
            }
        });
    }

    /// A group action (accept, decline, leave); the list reloads once it
    /// ran.
    pub(crate) fn spawn_group_request(&self, request: IpcRequest, context: &'static str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(request).await {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: format!("Done: {context}"),
                        level: ToastLevel::Info,
                    });
                    let _ = tx.send(Action::ShowGroupDms);
                }
                Err(e) => report_failure(&tx, context, e),
            }
        });
    }
}
