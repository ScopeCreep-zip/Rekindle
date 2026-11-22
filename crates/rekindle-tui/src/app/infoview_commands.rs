//! The Infoview's request from the TUI (plan E7.15 IV7): `InfoviewGet` for
//! the explicit subject of what is highlighted, from `TUI_WINDOW`. The Host
//! composes every section; later changes arrive as `InfoviewEvent::Updated`
//! to that window.

use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::infoview::{InfoviewSubject, InfoviewView, TUI_WINDOW};

use super::super::action::{Action, CommandResult};
use super::commands::report_failure;
use super::App;

impl App {
    /// `subject`'s Infoview, as the Host composes it.
    pub(crate) fn load_infoview(&self, subject: InfoviewSubject) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let request = IpcRequest::InfoviewGet {
            subject,
            window: TUI_WINDOW.to_owned(),
        };
        tokio::spawn(async move {
            match client.request_ok(request).await {
                Ok(v) => match serde_json::from_value::<InfoviewView>(v) {
                    Ok(view) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::InfoviewLoaded { view },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "infoview parse".into(),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => report_failure(&tx, "infoview", e),
            }
        });
    }

    /// Where an Infoview sits: under the dashboard, a friend or a
    /// community.
    pub(crate) fn infoview_breadcrumb(&self, subject: &InfoviewSubject) -> String {
        match subject {
            InfoviewSubject::Own | InfoviewSubject::Selection => "Dashboard > Infoview".into(),
            InfoviewSubject::Friend { public_key } => format!(
                "Friends > {} > Infoview",
                crate::helpers::abbreviate_key(public_key)
            ),
            InfoviewSubject::FriendGroup { group_id: Some(id) } => {
                format!("Friends > group {id} > Infoview")
            }
            InfoviewSubject::FriendGroup { group_id: None } => {
                "Friends > default group > Infoview".into()
            }
            InfoviewSubject::Community { community } => {
                format!("{} / Infoview", self.community_name(community))
            }
        }
    }
}
