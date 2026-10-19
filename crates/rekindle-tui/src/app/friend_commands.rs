//! Friend and DM requests from the TUI (plan E2.1): the buddy list with the
//! requests waiting for us, our blocks and open invites, answering
//! requests, removing and blocking, and the DM inbox. The host reviews
//! what needs it (`report_failure` shows the review).

use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::display as dt;
use rekindle_types::dto::dm::DmInboxEntry;
use rekindle_types::dto::friends::{BlockedUser, Friend, OutgoingInvite, PendingRequest};

use super::super::action::{Action, CommandResult, ToastLevel};
use super::commands::report_failure;
use super::App;

impl App {
    /// The buddy list, then the requests, blocks and invites beside it.
    pub(crate) fn load_friend_list(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::FriendList).await {
                Ok(value) => match serde_json::from_value::<Vec<Friend>>(value) {
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
                        return;
                    }
                },
                Err(e) => {
                    report_failure(&tx, "friend list", e);
                    return;
                }
            }
            let requests = client
                .request_ok(IpcRequest::FriendRequests)
                .await
                .ok()
                .and_then(|v| serde_json::from_value::<Vec<PendingRequest>>(v).ok())
                .unwrap_or_default();
            let blocked = client
                .request_ok(IpcRequest::FriendBlockedList)
                .await
                .ok()
                .and_then(|v| serde_json::from_value::<Vec<BlockedUser>>(v).ok())
                .unwrap_or_default();
            let invites = client
                .request_ok(IpcRequest::FriendInviteList)
                .await
                .ok()
                .and_then(|v| serde_json::from_value::<Vec<OutgoingInvite>>(v).ok())
                .unwrap_or_default();
            let _ = tx.send(Action::CommandComplete(Box::new(
                CommandResult::FriendExtrasLoaded {
                    requests,
                    blocked,
                    invites,
                },
            )));
        });
    }

    /// The latest message of each conversation, as inbox threads.
    pub(crate) fn load_dm_inbox(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::DmInbox { limit: 50 }).await {
                Ok(value) => match serde_json::from_value::<Vec<DmInboxEntry>>(value) {
                    Ok(entries) => {
                        let threads = entries.into_iter().map(inbox_thread).collect();
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::DmInboxLoaded { threads },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "dm inbox parse".into(),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => report_failure(&tx, "dm inbox", e),
            }
        });
    }

    pub(crate) fn spawn_accept_friend(&self, public_key: String, display_name: String) {
        let label = format!("Accepted {}", crate::helpers::abbreviate_key(&public_key));
        self.spawn_toasting(
            IpcRequest::FriendAccept {
                public_key,
                display_name,
            },
            "accept friend",
            label,
            ToastLevel::Success,
        );
    }

    pub(crate) fn spawn_reject_friend(&self, public_key: String) {
        let label = format!("Rejected {}", crate::helpers::abbreviate_key(&public_key));
        self.spawn_toasting(
            IpcRequest::FriendReject { public_key },
            "reject friend",
            label,
            ToastLevel::Info,
        );
    }

    pub(crate) fn spawn_remove_friend(&self, peer_key: String) {
        let label = format!(
            "Removed friend {}",
            crate::helpers::abbreviate_key(&peer_key)
        );
        self.spawn_toasting(
            IpcRequest::FriendRemove {
                public_key: peer_key,
            },
            "remove friend",
            label,
            ToastLevel::Info,
        );
    }

    /// A friend action; the friend list reloads once it ran.
    pub(crate) fn spawn_friend_request(&self, request: IpcRequest, context: &'static str) {
        self.spawn_action(
            request,
            context,
            format!("Done: {context}"),
            ToastLevel::Info,
            true,
        );
    }

    fn spawn_toasting(
        &self,
        request: IpcRequest,
        context: &'static str,
        done: String,
        level: ToastLevel,
    ) {
        self.spawn_action(request, context, done, level, false);
    }

    fn spawn_action(
        &self,
        request: IpcRequest,
        context: &'static str,
        done: String,
        level: ToastLevel,
        reload: bool,
    ) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(request).await {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: done,
                        level,
                    });
                    if reload {
                        let _ = tx.send(Action::ShowFriendList);
                    }
                }
                Err(e) => report_failure(&tx, context, e),
            }
        });
    }
}

/// An inbox entry as a one-message thread (its history loads with the
/// thread).
fn inbox_thread(entry: DmInboxEntry) -> dt::DmThreadDisplay {
    let from_peer = entry.sender_key == entry.peer_key;
    let peer_name = crate::helpers::abbreviate_key(&entry.peer_key);
    let timestamp = entry.timestamp.cast_unsigned();
    dt::DmThreadDisplay {
        unread_count: u32::from(from_peer && !entry.is_read),
        last_message_at: timestamp,
        messages: vec![dt::DmMessageDisplay {
            sender_name: if from_peer {
                peer_name.clone()
            } else {
                "you".into()
            },
            sender_key: entry.sender_key,
            body: entry.body,
            timestamp,
            is_self: !from_peer,
        }],
        peer_key: entry.peer_key,
        peer_name,
    }
}
