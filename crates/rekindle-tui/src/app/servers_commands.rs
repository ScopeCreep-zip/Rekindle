//! The Servers view's requests from the TUI (plan E7.2 G3): `ServersView`,
//! and `RecentServerRemove`, which answers the view as it now stands. The
//! Host composes every row and verdict; later changes arrive as
//! `ServersViewChanged`.

use std::sync::Arc;

use rekindle_ipc::protocol::{IpcRequest, ServersRequest};
use rekindle_types::dto::servers::ServersView;

use super::super::action::{Action, CommandResult};
use super::commands::report_failure;
use super::App;

impl App {
    /// The Servers view, as the Host composes it.
    pub(crate) fn load_servers_view(&self) {
        self.spawn_servers(IpcRequest::Servers(ServersRequest::View), "servers");
    }

    /// A request the Host answers with the Servers view.
    pub(crate) fn spawn_servers(&self, request: IpcRequest, context: &'static str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(request).await {
                Ok(v) => match serde_json::from_value::<ServersView>(v) {
                    Ok(view) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::ServersViewLoaded { view },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: format!("{context} parse"),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => report_failure(&tx, context, e),
            }
        });
    }
}
