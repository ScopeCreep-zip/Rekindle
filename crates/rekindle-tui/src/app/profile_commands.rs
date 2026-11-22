//! Profile and favourite-server requests from the TUI (plan E7.2): a
//! friend's bio and shared game time or our own, and our favourite game
//! servers. The Host decides everything shown; these only ask.

use std::sync::Arc;

use rekindle_ipc::protocol::{IpcRequest, ServersRequest};
use rekindle_types::dto::presence::{FavouriteServerView, GameTotalView};

use super::super::action::{Action, CommandResult};
use super::commands::report_failure;
use super::App;

impl App {
    /// A friend's bio and the totals they last shared, or ours.
    pub(crate) fn load_profile(&self, peer_key: Option<String>) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let (bio_req, totals_req) = match &peer_key {
                Some(key) => (
                    IpcRequest::FriendBioGet {
                        public_key: key.clone(),
                    },
                    IpcRequest::FriendGameTotalsGet {
                        public_key: key.clone(),
                    },
                ),
                None => (IpcRequest::ProfileBioGet, IpcRequest::GameTotalsGet),
            };
            let bio = match client.request_ok(bio_req).await {
                Ok(v) => serde_json::from_value::<Option<String>>(v),
                Err(e) => return report_failure(&tx, "profile bio", e),
            };
            let totals = match client.request_ok(totals_req).await {
                Ok(v) => serde_json::from_value::<Vec<GameTotalView>>(v),
                Err(e) => return report_failure(&tx, "game time", e),
            };
            match (bio, totals) {
                (Ok(bio), Ok(totals)) => {
                    let _ = tx.send(Action::CommandComplete(Box::new(
                        CommandResult::ProfileLoaded {
                            peer_key,
                            bio,
                            totals,
                        },
                    )));
                }
                (Err(e), _) | (_, Err(e)) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "profile parse".into(),
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    /// Our favourite servers.
    pub(crate) fn load_favourites(&self) {
        self.spawn_favourites(
            IpcRequest::Servers(ServersRequest::FavouritesGet),
            "favourite servers",
        );
    }

    /// A favourites request; each answers the list as it now stands.
    pub(crate) fn spawn_favourites(&self, request: IpcRequest, context: &'static str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(request).await {
                Ok(v) => match serde_json::from_value::<Vec<FavouriteServerView>>(v) {
                    Ok(servers) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::FavouritesLoaded { servers },
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
