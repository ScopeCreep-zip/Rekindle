//! Async IPC command spawning — all daemon requests from the TUI.

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
            if let Ok(value) = client.request_ok(IpcRequest::Status).await {
                if let Ok(snapshot) = serde_json::from_value::<dt::StatusSnapshot>(value) {
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::StatusLoaded { snapshot })));
                }
            }
            if let Ok(value) = client.request_ok(IpcRequest::NetworkPeers).await {
                if let Ok(peers) = serde_json::from_value::<Vec<dt::PeerSnapshot>>(value) {
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::PeerListLoaded { peers })));
                }
            }
            if let Ok(value) = client.request_ok(IpcRequest::CommunityList).await {
                if let Ok(communities) = serde_json::from_value::<Vec<dt::CommunityOverview>>(value) {
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::CommunityListLoaded { communities })));
                }
            }
            if let Ok(value) = client.request_ok(IpcRequest::IdentityShow).await {
                let public_key = value.get("public_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let display_name = value.get("display_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let profile_dht_key = value.get("profile_dht_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let mailbox_dht_key = value.get("mailbox_dht_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let friend_list_dht_key = value.get("friend_list_dht_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let friend_inbox_key = value.get("friend_inbox_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !public_key.is_empty() {
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::IdentityLoaded {
                        public_key, display_name,
                        profile_dht_key, mailbox_dht_key,
                        friend_list_dht_key, friend_inbox_key,
                    })));
                }
            }
            if let Ok(Ok(friends)) = client.request_ok(IpcRequest::FriendList).await.map(serde_json::from_value::<Vec<dt::FriendDisplay>>) {
                let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::FriendListLoaded { friends })));
            }
        });
    }

    pub(crate) fn load_channel_history(&self, community: &str, channel: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        let channel = channel.to_string();
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::ChannelHistory { community: community.clone(), channel: channel.clone(), limit: 50 }).await {
                Ok(value) => {
                    if let Ok(messages) = serde_json::from_value::<Vec<dt::DecryptedMessageDisplay>>(value) {
                        let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::ChannelHistoryLoaded { community, channel, messages })));
                    }
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "channel history".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn load_dm_inbox(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::DmInbox { limit: 50 }).await {
                Ok(value) => {
                    let threads = serde_json::from_value::<Vec<dt::DmThreadDisplay>>(value).unwrap_or_default();
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::DmInboxLoaded { threads })));
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "dm inbox".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn load_dm_thread(&self, peer_key: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let peer_key = peer_key.to_string();
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::DmThread { peer_key: peer_key.clone(), limit: 50 }).await {
                Ok(value) => {
                    let messages = serde_json::from_value::<Vec<dt::DmMessageDisplay>>(value).unwrap_or_default();
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::DmThreadLoaded { peer_key, messages })));
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "dm thread".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn load_friend_list(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::FriendList).await {
                Ok(value) => {
                    if let Ok(friends) = serde_json::from_value::<Vec<dt::FriendDisplay>>(value) {
                        let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::FriendListLoaded { friends })));
                    }
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "friend list".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn load_community_info(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::CommunityInfo { governance_key: community }).await {
                Ok(value) => {
                    if let Ok(detail) = serde_json::from_value::<dt::CommunityDetail>(value) {
                        let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::CommunityInfoLoaded { detail })));
                    }
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "community info".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_send_channel_message(&self, community: &str, channel: &str, text: String, reply_to: Option<String>) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        let channel = channel.to_string();
        tokio::spawn(async move {
            let reply = reply_to.and_then(|r| r.parse::<u64>().ok());
            match client.request_ok(IpcRequest::ChannelSend { community, channel, body: text, reply_to: reply }).await {
                Ok(value) => {
                    let msg_id = value.get("message_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::MessageSent { message_id: msg_id })));
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed { context: "send message".into(), error: e.to_string() });
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::SendFailed)));
                }
            }
        });
    }

    pub(crate) fn spawn_send_dm(&self, peer_key: String, text: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::DmSend { peer_key, body: text }).await {
                Ok(value) => {
                    let msg_id = value.get("message_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::MessageSent { message_id: msg_id })));
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed { context: "send DM".into(), error: e.to_string() });
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::SendFailed)));
                }
            }
        });
    }

    pub(crate) fn spawn_edit_message(&self, community: String, channel: String, message_id: String, new_body: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::MessageEdit { community, channel, message_id, new_body }).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Message edited".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "edit message".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_delete_message(&self, community: String, channel: String, message_id: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::MessageDelete { community, channel, message_id }).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Message deleted".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "delete message".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_accept_friend(&self, request_id: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let short = crate::v2::helpers::abbreviate_key(&request_id);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::FriendAccept { public_key: request_id }).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: format!("Accepted {short}"), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "accept friend".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_reject_friend(&self, request_id: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let short = crate::v2::helpers::abbreviate_key(&request_id);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::FriendReject { public_key: request_id }).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: format!("Rejected {short}"), level: ToastLevel::Info }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "reject friend".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_channel_typing(&self, community: String, channel: String) {
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let _ = client.request_ok(IpcRequest::ChannelTyping { community, channel }).await;
        });
    }

    pub(crate) fn spawn_dm_typing(&self, peer_key: String) {
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let _ = client.request_ok(IpcRequest::DmTyping { peer_key, typing: true }).await;
        });
    }
}
