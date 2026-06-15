//! Async IPC command spawning — all daemon requests from the TUI.

use std::sync::Arc;
use crate::v2::prelude::{ChatRequest, DaemonRequest, LifecycleRequest};
use rekindle_types::display as dt;
use super::super::action::{Action, CommandResult, ToastLevel};
use super::App;

impl App {
    /// Lightweight status-only refresh — one IPC round-trip.
    /// Called every 2 seconds on the Dashboard view to keep the
    /// status panel fresh (peers, attachment, route, uptime correction).
    pub(crate) fn load_status_only(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            if let Ok(value) = client.request_ok(DaemonRequest::Lifecycle(LifecycleRequest::Status)).await {
                if let Ok(snapshot) = serde_json::from_value::<dt::StatusSnapshot>(value) {
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::StatusLoaded { snapshot })));
                }
            }
        });
    }

    /// Full dashboard refresh — 5 IPC round-trips (Status, Peers,
    /// Communities, Identity, Friends). Called every 30 seconds and
    /// on view transitions.
    pub(crate) fn load_dashboard_data(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            if let Ok(value) = client.request_ok(DaemonRequest::Lifecycle(LifecycleRequest::Status)).await {
                if let Ok(snapshot) = serde_json::from_value::<dt::StatusSnapshot>(value) {
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::StatusLoaded { snapshot })));
                }
            }
            if let Ok(value) = client.request_ok(DaemonRequest::Lifecycle(LifecycleRequest::NetworkPeers)).await {
                if let Ok(peers) = serde_json::from_value::<Vec<dt::PeerSnapshot>>(value) {
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::PeerListLoaded { peers })));
                }
            }
            if let Ok(value) = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityList)).await {
                if let Ok(communities) = serde_json::from_value::<Vec<dt::CommunityOverview>>(value) {
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::CommunityListLoaded { communities })));
                }
            }
            if let Ok(value) = client.request_ok(DaemonRequest::Chat(ChatRequest::IdentityShow)).await {
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
            if let Ok(Ok(friends)) = client.request_ok(DaemonRequest::Chat(ChatRequest::FriendList)).await.map(serde_json::from_value::<Vec<dt::FriendDisplay>>) {
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::ChannelHistory { community: community.clone(), channel: channel.clone(), limit: 50 })).await {
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::DmInbox { limit: 50 })).await {
                Ok(value) => {
                    let threads = match serde_json::from_value::<Vec<dt::DmThreadDisplay>>(value.clone()) {
                        Ok(t) => {
                            tracing::debug!(thread_count = t.len(), "TUI: dm inbox deserialized");
                            t
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, raw_json = %value, "TUI: dm inbox deserialize FAILED — showing empty inbox");
                            Vec::new()
                        }
                    };
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::DmThread { peer_key: peer_key.clone(), limit: 50 })).await {
                Ok(value) => {
                    let messages = match serde_json::from_value::<Vec<dt::DmMessageDisplay>>(value.clone()) {
                        Ok(m) => {
                            tracing::debug!(peer = &peer_key[..12.min(peer_key.len())], msg_count = m.len(), "TUI: dm thread deserialized");
                            m
                        }
                        Err(e) => {
                            tracing::warn!(peer = &peer_key[..12.min(peer_key.len())], error = %e, raw_json = %value, "TUI: dm thread deserialize FAILED — showing empty thread");
                            Vec::new()
                        }
                    };
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::FriendList)).await {
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityInfo { governance_key: community })).await {
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::ChannelSend { community, channel, body: text, reply_to: reply, client_msg_id: None })).await {
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::DmSend { peer_key, body: text })).await {
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::MessageEdit { community, channel, message_id, new_body })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Message edited".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "edit message".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_delete_message(&self, community: String, channel: String, message_id: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::MessageDelete { community, channel, message_id })).await {
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::FriendAccept { public_key: request_id })).await {
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
            match client.request_ok(DaemonRequest::Chat(ChatRequest::FriendReject { public_key: request_id })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: format!("Rejected {short}"), level: ToastLevel::Info }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "reject friend".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_kick_member(&self, community: String, pseudonym: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::Kick { community: community.clone(), target_pseudonym: pseudonym })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Member kicked".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "kick".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_ban_member(&self, community: String, pseudonym: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::Ban { community: community.clone(), target_pseudonym: pseudonym, reason: None })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Member banned".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "ban".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_unban_member(&self, community: String, pseudonym: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::Unban { community: community.clone(), target_pseudonym: pseudonym })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Member unbanned".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "unban".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_timeout_member(&self, community: String, pseudonym: String, duration_secs: u64) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::Timeout { community: community.clone(), target_pseudonym: pseudonym, duration_seconds: duration_secs, reason: None })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: format!("Member timed out for {duration_secs}s"), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "timeout".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_approve_member(&self, community: String, pseudonym: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityApprove { governance_key: community.clone(), member_pseudonym: pseudonym })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Member approved".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "approve".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_reject_member(&self, community: String, pseudonym: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityReject { governance_key: community.clone(), member_pseudonym: pseudonym, reason: String::new() })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Member rejected".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "reject".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_create_invite(&self, community: String, max_uses: u32, expires_seconds: Option<u64>) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::InviteCreate { community: community.clone(), max_uses, expires_seconds })).await {
                Ok(value) => {
                    let code = value.get("code").or_else(|| value.get("invite_code")).and_then(|v| v.as_str()).unwrap_or("created");
                    let _ = tx.send(Action::ShowToast { message: format!("Invite created: {code}"), level: ToastLevel::Success });
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "create invite".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_revoke_invite(&self, community: String, invite_code: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::InviteRevoke { community: community.clone(), invite_code: invite_code.clone() })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: format!("Invite {invite_code} revoked"), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "revoke invite".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn load_thread_messages(&self, thread_id: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let thread_id = thread_id.to_string();
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::ThreadHistory {
                thread_id: thread_id.clone(),
                limit: 50,
            })).await {
                Ok(value) => {
                    let messages = serde_json::from_value::<Vec<dt::DecryptedMessageDisplay>>(value).unwrap_or_default();
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::ThreadMessagesLoaded {
                        thread_id, messages,
                    })));
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "thread history".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_add_reaction(&self, community: String, channel: String, message_id: String, emoji: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::ReactionAdd { community, channel, message_id, emoji: emoji.clone() })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: format!("{emoji} added"), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "add reaction".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_remove_reaction(&self, community: String, channel: String, message_id: String, emoji: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::ReactionRemove { community, channel, message_id, emoji })).await {
                Ok(_) => {}
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "remove reaction".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn spawn_unpin_message(&self, community: String, channel: String, message_id: String) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::PinRemove { community, channel, message_id })).await {
                Ok(_) => { let _ = tx.send(Action::ShowToast { message: "Unpinned".into(), level: ToastLevel::Success }); }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "unpin".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn load_pins(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::PinList { community })).await {
                Ok(value) => {
                    let pins = value.as_array().map(|arr| {
                        arr.iter().filter_map(|p| {
                            Some(crate::v2::tui::components::pins_panel::PinDisplay {
                                message_id: p.get("messageId").or_else(|| p.get("message_id")).and_then(|v| v.as_str())?.to_string(),
                                channel_id: p.get("channelId").or_else(|| p.get("channel_id")).and_then(|v| v.as_str())?.to_string(),
                                pinned_by: p.get("pinnedBy").or_else(|| p.get("pinned_by")).and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                                pinned_at: p.get("pinnedAt").or_else(|| p.get("pinned_at")).and_then(|v| v.as_u64()).unwrap_or(0),
                                body_preview: "(pinned message)".to_string(),
                            })
                        }).collect()
                    }).unwrap_or_default();
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::PinsLoaded { pins })));
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "pin list".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn load_onboarding(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            let config = client.request_ok(DaemonRequest::Chat(ChatRequest::OnboardingConfigGet {
                community: community.clone(),
            })).await.ok().and_then(|v| serde_json::from_value(v).ok());
            let welcome = client.request_ok(DaemonRequest::Chat(ChatRequest::WelcomeScreenGet {
                community,
            })).await.ok().and_then(|v| serde_json::from_value(v).ok());
            let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::OnboardingLoaded { config, welcome })));
        });
    }

    pub(crate) fn load_events(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::EventList { community })).await {
                Ok(value) => {
                    let events = value.as_array().cloned().unwrap_or_default();
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::EventsLoaded { events })));
                }
                Err(e) => { let _ = tx.send(Action::CommandFailed { context: "event list".into(), error: e.to_string() }); }
            }
        });
    }

    pub(crate) fn load_invites(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            match client.request_ok(DaemonRequest::Chat(ChatRequest::InviteList {
                community,
            })).await {
                Ok(value) => {
                    let invites = value.as_array().cloned().unwrap_or_default();
                    let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::InvitesLoaded { invites })));
                }
                Err(e) => {
                    let _ = tx.send(Action::CommandFailed { context: "invite list".into(), error: e.to_string() });
                }
            }
        });
    }

    pub(crate) fn load_moderation_data(&self, community: &str) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        let community = community.to_string();
        tokio::spawn(async move {
            let detail = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityInfo {
                governance_key: community.clone(),
            })).await.ok().and_then(|v| serde_json::from_value::<rekindle_types::display::CommunityDetail>(v).ok());
            let bans = client.request_ok(DaemonRequest::Chat(ChatRequest::BanList {
                community: community.clone(),
            })).await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            let pending = client.request_ok(DaemonRequest::Chat(ChatRequest::CommunityPendingMembers {
                governance_key: community,
            })).await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            if let Some(detail) = detail {
                let _ = tx.send(Action::CommandComplete(Box::new(CommandResult::ModerationDataLoaded {
                    detail, bans, pending,
                })));
            }
        });
    }

    pub(crate) fn spawn_channel_typing(&self, community: String, channel: String) {
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let _ = client.request_ok(DaemonRequest::Chat(ChatRequest::ChannelTyping { community, channel })).await;
        });
    }

    pub(crate) fn spawn_dm_typing(&self, peer_key: String) {
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let _ = client.request_ok(DaemonRequest::Chat(ChatRequest::DmTyping { peer_key, typing: true })).await;
        });
    }
}
