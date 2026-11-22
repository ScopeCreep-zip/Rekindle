//! The activity feed's request from the TUI (plan E7.15, TU1(a)): one page
//! of `ActivityList`, as the CLI's `rekindle activity` asks it. The Host
//! derives and names every entry; this only asks.

use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::activity::ActivityListItem;

use super::super::action::{Action, CommandResult};
use super::commands::report_failure;
use super::App;
use crate::views::activity::ACTIVITY_PAGE;

impl App {
    /// A page of the feed: the first (`before` `None`) or the one older
    /// than row `before`. A failure is the Host's, shown as a toast.
    pub(crate) fn load_activity(&self, before: Option<i64>) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let request = IpcRequest::ActivityList {
                before,
                limit: ACTIVITY_PAGE,
            };
            match client.request_ok(request).await {
                Ok(v) => match serde_json::from_value::<Vec<ActivityListItem>>(v) {
                    Ok(entries) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::ActivityLoaded { before, entries },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "activity parse".into(),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => report_failure(&tx, "activity", e),
            }
        });
    }
}
