//! Game requests from the TUI (plan E7.15 IV3): the games we can launch
//! (`InstalledGamesGet`) and the one launch path (`GameLaunch`). The Host
//! decides: a launch to a peer's server comes back as its review, shown by
//! the intent overlay (`report_failure`); a refusal is an error toast.

use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::presence::InstalledGameView;
use rekindle_types::game_server::{JoinSource, ServerAddress};

use super::super::action::{Action, CommandResult, ToastLevel};
use super::commands::report_failure;
use super::App;

impl App {
    /// The games we can launch, as the Host lists them.
    pub(crate) fn load_installed_games(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::InstalledGamesGet).await {
                Ok(value) => match serde_json::from_value::<Vec<InstalledGameView>>(value) {
                    Ok(games) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::InstalledGamesLoaded { games },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "installed games parse".into(),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => report_failure(&tx, "installed games", e),
            }
        });
    }

    /// `GameLaunch` as the view asked for it.
    pub(crate) fn spawn_game_launch(
        &self,
        game_id: u32,
        server: Option<ServerAddress>,
        source: JoinSource,
    ) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::GameLaunch {
                    game_id,
                    server,
                    source,
                })
                .await
            {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: "Game launched".into(),
                        level: ToastLevel::Success,
                    });
                }
                Err(e) => report_failure(&tx, "launch game", e),
            }
        });
    }
}
