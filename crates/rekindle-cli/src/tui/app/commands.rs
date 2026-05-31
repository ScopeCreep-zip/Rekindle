//! Async IPC command spawning — all daemon requests from the TUI.
//!
//! Every method spawns a tokio task that sends an `IpcRequest` to the daemon
//! via `DaemonClient`, deserializes the response into the appropriate
//! `rekindle_types::display` struct, and sends a `CommandResult` back through
//! the action channel.

use std::sync::Arc;

use rekindle_node::ipc::protocol::IpcRequest;
use rekindle_types::display as dt;

use super::super::action::{Action, CommandResult, ToastLevel};
use super::App;

impl App {
    pub(crate) fn load_dashboard_data(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            // Status snapshot (compact + checks + subscription health)
            match client.request_ok(IpcRequest::Status).await {
                Ok(value) => match serde_json::from_value::<dt::StatusSnapshot>(value.clone()) {
                    Ok(snapshot) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::StatusLoaded { snapshot },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "status parse".into(),
                            error: format!("{e}: {value}"),
                        });
                    }
                },
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "status".into(),
                        error: e.to_string(),
                    });
                }
            }

            // Peer list
            match client.request_ok(IpcRequest::NetworkPeers).await {
                Ok(value) => match serde_json::from_value::<Vec<dt::PeerSnapshot>>(value.clone()) {
                    Ok(peers) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::PeerListLoaded { peers },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "peers parse".into(),
                            error: format!("{e}: {value}"),
                        });
                    }
                },
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "peers".into(),
                        error: e.to_string(),
                    });
                }
            }

            // Community list
            match client.request_ok(IpcRequest::CommunityList).await {
                Ok(value) => {
                    match serde_json::from_value::<Vec<dt::CommunityOverview>>(value.clone()) {
                        Ok(communities) => {
                            let _ = tx.send(Action::CommandComplete(Box::new(
                                CommandResult::CommunityListLoaded { communities },
                            )));
                        }
                        Err(e) => {
                            let _ = tx.send(Action::CommandFailed {
                                context: "communities parse".into(),
                                error: format!("{e}: {value}"),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "communities".into(),
                        error: e.to_string(),
                    });
                }
            }

            // Identity (for dashboard identity panel)
            if let Ok(value) = client.request_ok(IpcRequest::IdentityShow).await {
                let public_key = value
                    .get("public_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let display_name = value
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !public_key.is_empty() {
                    let _ = tx.send(Action::CommandComplete(Box::new(
                        CommandResult::IdentityLoaded {
                            public_key,
                            display_name,
                        },
                    )));
                }
            }

            // Friend list (for dashboard friends panel)
            if let Ok(Ok(friends)) = client
                .request_ok(IpcRequest::FriendList)
                .await
                .map(serde_json::from_value::<Vec<dt::FriendDisplay>>)
            {
                let _ = tx.send(Action::CommandComplete(Box::new(
                    CommandResult::FriendListLoaded { friends },
                )));
            }
        });
    }

    pub(crate) fn load_channel_history(&self, community: &str, channel: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        let channel = channel.to_string();
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::ChannelHistory {
                    community: community.clone(),
                    channel: channel.clone(),
                    limit: 50,
                })
                .await
            {
                Ok(value) => {
                    match serde_json::from_value::<Vec<dt::DecryptedMessageDisplay>>(value) {
                        Ok(messages) => {
                            let _ = tx.send(Action::CommandComplete(Box::new(
                                CommandResult::ChannelHistoryLoaded {
                                    community,
                                    channel,
                                    messages,
                                },
                            )));
                        }
                        Err(e) => {
                            let _ = tx.send(Action::CommandFailed {
                                context: "channel history parse".into(),
                                error: e.to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "channel history".into(),
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(crate) fn load_dm_inbox(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::DmInbox { limit: 50 }).await {
                Ok(value) => match serde_json::from_value::<Vec<dt::DmThreadDisplay>>(value) {
                    Ok(threads) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::DmInboxLoaded { threads },
                        )));
                    }
                    Err(_) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::DmInboxLoaded {
                                threads: Vec::new(),
                            },
                        )));
                    }
                },
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "dm inbox".into(),
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(crate) fn load_friend_list(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::FriendList).await {
                Ok(value) => match serde_json::from_value::<Vec<dt::FriendDisplay>>(value) {
                    Ok(friends) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::FriendListLoaded { friends },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "friend list parse".into(),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "friend list".into(),
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(crate) fn load_community_info(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::CommunityInfo {
                    governance_key: community,
                })
                .await
            {
                Ok(value) => match serde_json::from_value::<dt::CommunityDetail>(value) {
                    Ok(detail) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::CommunityInfoLoaded { detail },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "community info parse".into(),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "community info".into(),
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(crate) fn spawn_send_channel_message(
        &self,
        community: &str,
        channel: &str,
        text: String,
        reply_to: Option<String>,
    ) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        let channel_clone = channel.to_string();
        tokio::spawn(async move {
            let reply = reply_to.and_then(|r| r.parse::<u64>().ok());
            match client
                .request_ok(IpcRequest::ChannelSend {
                    community,
                    channel: channel_clone.clone(),
                    body: text,
                    reply_to: reply,
                })
                .await
            {
                Ok(value) => {
                    let msg_id = value
                        .get("message_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let _ = tx.send(Action::CommandComplete(Box::new(
                        CommandResult::MessageSent { message_id: msg_id },
                    )));
                    let _ = tx.send(Action::ShowToast {
                        message: format!("Sent to #{channel_clone}"),
                        level: ToastLevel::Success,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: format!("send to #{channel_clone}"),
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(crate) fn spawn_send_dm(&self, peer_key: String, text: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::DmSend {
                    peer_key: peer_key.clone(),
                    body: text,
                })
                .await
            {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: format!(
                            "DM sent to {}",
                            crate::helpers::abbreviate_key(&peer_key)
                        ),
                        level: ToastLevel::Success,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "send DM".into(),
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(crate) fn spawn_accept_friend(&self, request_id: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::FriendAccept {
                    public_key: request_id.clone(),
                })
                .await
            {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: format!(
                            "Accepted {}",
                            crate::helpers::abbreviate_key(&request_id)
                        ),
                        level: ToastLevel::Success,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "accept friend".into(),
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(crate) fn spawn_reject_friend(&self, request_id: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::FriendReject {
                    public_key: request_id.clone(),
                })
                .await
            {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: format!(
                            "Rejected {}",
                            crate::helpers::abbreviate_key(&request_id)
                        ),
                        level: ToastLevel::Info,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed {
                        context: "reject friend".into(),
                        error: e.to_string(),
                    });
                }
            }
        });
    }
}
