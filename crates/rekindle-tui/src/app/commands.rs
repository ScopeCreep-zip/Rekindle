//! Async IPC command spawning — all daemon requests from the TUI.
//!
//! Every method spawns a tokio task that sends an `IpcRequest` to the daemon
//! via `DaemonClient`, deserializes the response into the appropriate
//! `rekindle_types::display` struct, and sends a `CommandResult` back through
//! the action channel.

use std::sync::Arc;

use rekindle_ipc::protocol::{ChannelMessageRequest, LifecycleRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::display as dt;
use rekindle_types::dto::channel_message::ChannelMessageView;
use rekindle_types::dto::community::CommunityMemberView;

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
                Err(e) => report_failure(&tx, "status", e),
            }

            // Community list
            match client
                .request_ok(IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityList,
                )))
                .await
            {
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
                Err(e) => report_failure(&tx, "communities", e),
            }

            // Identity (for dashboard identity panel)
            if let Ok(value) = client.request_ok(IpcRequest::IdentityShow).await {
                if let Ok(Some(shown)) = serde_json::from_value::<
                    Option<rekindle_types::dto::identity::IdentityShown>,
                >(value)
                {
                    let _ = tx.send(Action::CommandComplete(Box::new(
                        CommandResult::IdentityLoaded {
                            public_key: shown.public_key,
                            display_name: shown.display_name,
                        },
                    )));
                }
            }

            // Our friend code and its QR (plan E2.5).
            if let Ok(Ok(view)) = client
                .request_ok(IpcRequest::FriendCode)
                .await
                .map(serde_json::from_value::<rekindle_types::dto::friends::FriendCodeView>)
            {
                let _ = tx.send(Action::CommandComplete(Box::new(
                    CommandResult::FriendCodeLoaded { view },
                )));
            }

            // Our own presence (plan E1.2), live after this from `SelfChanged`.
            if let Ok(Ok(view)) = client
                .request_ok(IpcRequest::PresenceGet)
                .await
                .map(serde_json::from_value::<rekindle_types::dto::presence::OwnPresenceView>)
            {
                let _ = tx.send(Action::CommandComplete(Box::new(
                    CommandResult::OwnPresenceLoaded { view },
                )));
            }

            // The network bars now (plan E7.2), live after this from
            // `NetworkEvent::Strength`: a TUI started after the last change
            // would otherwise show none.
            if let Some(bars) = client
                .request_ok(IpcRequest::NetworkStatus)
                .await
                .ok()
                .and_then(|v| v.get("bars").and_then(serde_json::Value::as_u64))
                .and_then(|b| u8::try_from(b).ok())
            {
                let _ = tx.send(Action::CommandComplete(Box::new(
                    CommandResult::NetworkBarsLoaded { bars },
                )));
            }

            // Friend list (for dashboard friends panel)
            if let Ok(Ok(friends)) = client
                .request_ok(IpcRequest::FriendList)
                .await
                .map(serde_json::from_value::<Vec<rekindle_types::dto::friends::Friend>>)
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
                .request_ok(IpcRequest::Community(CommunityRequest::Messages(
                    ChannelMessageRequest::ChannelMessages {
                        channel: channel.clone(),
                        limit: 50,
                    },
                )))
                .await
            {
                Ok(value) => match serde_json::from_value::<Vec<ChannelMessageView>>(value) {
                    Ok(views) => {
                        let messages = views.into_iter().map(message_row).collect();
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
                },
                Err(e) => report_failure(&tx, "channel history", e),
            }
        });
    }

    /// A community's member list, grouped by the Host.
    pub(crate) fn load_community_members(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityMembers {
                        anchor: community.clone(),
                    },
                )))
                .await
            {
                Ok(value) => match serde_json::from_value::<Vec<CommunityMemberView>>(value) {
                    Ok(members) => {
                        let _ = tx.send(Action::CommandComplete(Box::new(
                            CommandResult::CommunityMembersLoaded { community, members },
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "community members parse".into(),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => report_failure(&tx, "community members", e),
            }
        });
    }

    pub(crate) fn load_community_info(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityInfo { anchor: community },
                )))
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
                Err(e) => report_failure(&tx, "community info", e),
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
            match client
                .request_ok(IpcRequest::Community(CommunityRequest::Messages(
                    ChannelMessageRequest::ChannelSend {
                        community,
                        channel: channel_clone.clone(),
                        body: text,
                        reply_to,
                    },
                )))
                .await
            {
                Ok(value) => {
                    let msg_id = value
                        .get("messageId")
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
                    reply_to: None,
                })
                .await
            {
                // Committed: the thread reloads with its queued echo, and
                // the outbox's states then stream (plan E2.5, SR2).
                Ok(_) => {
                    let _ = tx.send(Action::LoadDmHistory { peer_key });
                }
                Err(e) => report_failure(&tx, "send DM", e),
            }
        });
    }

    pub(crate) fn spawn_leave_community(&self, anchor: String, name: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityLeave {
                        anchor: anchor.clone(),
                    },
                )))
                .await
            {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: format!("Left '{name}'"),
                        level: ToastLevel::Info,
                    });
                    let _ = tx.send(Action::ShowDashboard);
                }
                Err(e) => report_failure(&tx, "leave community", e),
            }
        });
    }

    pub(crate) fn spawn_leave_voice(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::VoiceLeave).await {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: "Left voice channel".into(),
                        level: ToastLevel::Info,
                    });
                    let _ = tx.send(Action::ShowDashboard);
                }
                Err(e) => report_failure(&tx, "leave voice channel", e),
            }
        });
    }

    /// Edit, delete or remove a channel message (plan E3.3 slice 9): the
    /// GUI's request as it is; a review the Host's gate asks for shows in
    /// the intent overlay and a refusal as an error toast. The change itself
    /// reaches the view as the Host's `Edited`/`Deleted` event.
    pub(crate) fn spawn_channel_change(
        &self,
        request: ChannelMessageRequest,
        context: &'static str,
        done: String,
    ) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client
                .request_ok(IpcRequest::Community(CommunityRequest::Messages(request)))
                .await
            {
                Ok(_) => {
                    let _ = tx.send(Action::ShowToast {
                        message: done,
                        level: ToastLevel::Success,
                    });
                }
                Err(e) => report_failure(&tx, context, e),
            }
        });
    }

    /// Commit a reviewed action with `proof`; a changed fact shows the fresh
    /// review, a wrong proof shows the same one again (it is still held).
    pub(crate) fn spawn_commit_intent(
        &self,
        review: rekindle_types::intent::IntentReview,
        proof: rekindle_types::intent::IntentProof,
    ) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.commit_intent(review.intent_id, proof).await {
                Ok(ran) => {
                    let _ = tx.send(Action::ShowToast {
                        message: format!(
                            "{:?} done ({:?})",
                            review.kind,
                            ran.reversibility.unwrap_or(review.reversibility)
                        ),
                        level: ToastLevel::Success,
                    });
                }
                // The gate's wrong-proof answer (401) shows the review again;
                // an action's own refusal ends it (IJ3).
                Err(rekindle_client::ClientError::Daemon {
                    code: 401, message, ..
                }) => {
                    let _ = tx.send(Action::ShowToast {
                        message,
                        level: ToastLevel::Warning,
                    });
                    let _ = tx.send(Action::ShowIntent(Box::new(review)));
                }
                Err(e) => report_failure(&tx, "confirm", e),
            }
        });
    }

    /// Drop a reviewed action.
    pub(crate) fn spawn_cancel_intent(&self, intent_id: rekindle_types::intent::IntentId) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            if let Err(e) = client.cancel_intent(intent_id).await {
                report_failure(&tx, "cancel", e);
            }
        });
    }
}

/// A request that did not run: the host's review, shown in the overlay, or
/// the error.
pub(super) fn report_failure(
    tx: &tokio::sync::mpsc::UnboundedSender<Action>,
    context: &str,
    error: rekindle_client::ClientError,
) {
    let action = match error {
        rekindle_client::ClientError::IntentRequired(review) => Action::ShowIntent(review),
        other => Action::CommandFailed {
            context: context.into(),
            error: other.to_string(),
        },
    };
    let _ = tx.send(action);
}

/// The message list's row for one message of the Host's channel history
/// (`ChannelMessages`). A body that did not open is shown as encrypted,
/// its generation unknown, as a live message without a body is.
fn message_row(view: ChannelMessageView) -> dt::DecryptedMessageDisplay {
    let encrypted = view.decryption_failed;
    dt::DecryptedMessageDisplay {
        message_id: view
            .server_message_id
            .unwrap_or_else(|| view.id.to_string()),
        sequence: 0,
        author_display_name: crate::helpers::abbreviate_key(&view.sender_id),
        author_pseudonym: view.sender_id,
        body: view.body,
        timestamp: u64::try_from(view.timestamp).unwrap_or(0),
        reply_to_message_id: None,
        mek_generation: 0,
        is_encrypted: encrypted,
        needs_mek: encrypted.then_some(0),
        automod_blurred: view.automod_blurred,
        is_own: view.is_own,
        edited: view.edited_at.is_some(),
        removed_by: view.removed_by,
    }
}
