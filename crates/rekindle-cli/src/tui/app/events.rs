//! Event → Action translation for terminal, transport, and subscription events.

use super::super::action::{Action, ToastLevel};
use super::super::event::Event;
use super::super::keybinds::KeymapContext;
use super::super::navigator::KeyResolution;
use super::App;

use rekindle_types::subscription_events::{
    SubscriptionEvent,
    ChannelMessageEvent, MembershipEvent, FriendEvent,
    CryptoEvent, VoiceEvent, GovernanceEvent, SocialEvent,
    NetworkEvent, SystemEvent,
};

impl App {
    /// Convert a terminal/transport event into an Action for the reducer.
    pub(crate) fn event_to_action(&mut self, event: Event) -> Option<Action> {
        match event {
            Event::Tick => Some(Action::Tick),
            Event::Render => Some(Action::Render),
            Event::Resize(w, h) => Some(Action::Resize(w, h)),
            Event::Key(key) => {
                self.idle_frames = 0;

                if self.search.visible {
                    return self.search.handle_key(key);
                }

                if self.confirm.visible {
                    use crossterm::event::KeyCode;
                    match key.code {
                        KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                            self.confirm.toggle_focus();
                            return None;
                        }
                        _ => {}
                    }
                }

                let use_unicode = self.theme.use_unicode();
                match self.nav.handle_key(key, &self.keymap, use_unicode) {
                    KeyResolution::Action(action) => Some(action),
                    KeyResolution::ForwardToView(key) => {
                        self.nav.current_view_mut().handle_focused_key(key)
                    }
                    KeyResolution::Consumed => None,
                    KeyResolution::ForceQuit => {
                        self.should_quit = true;
                        Some(Action::Render)
                    }
                }
            }
            Event::Mouse(mouse) => {
                self.idle_frames = 0;
                use crossterm::event::MouseEventKind;
                match mouse.kind {
                    MouseEventKind::ScrollDown => Some(Action::ScrollDown(3)),
                    MouseEventKind::ScrollUp => Some(Action::ScrollUp(3)),
                    MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                        if let Some(tab_idx) = self.nav.tab_bar.click_tab(
                            mouse.column, mouse.row, self.nav.tab_bar_row,
                        ) {
                            self.nav.tab_bar.select(tab_idx);
                            self.transition_to_selected_tab();
                            return Some(Action::Render);
                        }
                        self.nav.current_view_mut().handle_click(mouse.column, mouse.row)
                    }
                    _ => None,
                }
            }
            Event::Paste(text) => {
                if self.nav.input_mode() {
                    tracing::debug!(len = text.len(), "paste event");
                }
                None
            }
            Event::Init | Event::FocusGained | Event::FocusLost => None,
        }
    }

    /// Convert a daemon subscription event into a TUI action.
    ///
    /// Every `SubscriptionEvent` variant is explicitly matched — no wildcard.
    /// Each event maps to the most appropriate TUI response: render for visual
    /// updates, toast for user-facing notifications, both for important events.
    pub(crate) fn subscription_event_to_action(
        &mut self, event: SubscriptionEvent,
    ) -> Option<Action> {
        self.idle_frames = 0;

        match event {
            // ── Events forwarded to active view (real-time data push) ─────
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New { .. }) => {
                if self.nav.tab_bar.selected_id() != Some("communities") {
                    self.nav.tab_bar.increment_unread("communities");
                }
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived { ref peer_key, is_self: false, .. }) => {
                let short = crate::helpers::abbreviate_key(peer_key);
                self.notifications.push(format!("New DM from {short}"), ToastLevel::Info);
                if self.nav.tab_bar.selected_id() != Some("dms") {
                    self.nav.tab_bar.increment_unread("dms");
                }
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived { .. }) => {
                // Self-sent DM — no toast, no unread badge. The ○→● indicator is sufficient.
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Edited { .. } | ChannelMessageEvent::Deleted { .. })
            | SubscriptionEvent::Voice(
                VoiceEvent::ModeChanged { .. } | VoiceEvent::MuteChanged { .. }
                | VoiceEvent::DeafenChanged { .. } | VoiceEvent::RosterUpdated { .. }
            )
            | SubscriptionEvent::Network(NetworkEvent::AttachmentChanged { .. })
            | SubscriptionEvent::Presence(_)
            | SubscriptionEvent::Typing(_)
            | SubscriptionEvent::UnreadChanged { .. } => {
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Voice(VoiceEvent::Joined { ref pseudonym, ref channel, .. }) => {
                let short = crate::helpers::abbreviate_key(pseudonym);
                self.notifications.push(format!("{short} joined voice #{channel}"), ToastLevel::Info);
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Voice(VoiceEvent::Left { ref pseudonym, ref channel, .. }) => {
                let short = crate::helpers::abbreviate_key(pseudonym);
                self.notifications.push(format!("{short} left voice #{channel}"), ToastLevel::Info);
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Friend(FriendEvent::RequestReceived { ref display_name, .. }) => {
                self.notifications.push(format!("Friend request from {display_name}"), ToastLevel::Info);
                if self.nav.tab_bar.selected_id() != Some("friends") {
                    self.nav.tab_bar.increment_unread("friends");
                }
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Friend(FriendEvent::Accepted { .. }) => {
                self.notifications.push("Friend request accepted!".into(), ToastLevel::Success);
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Friend(FriendEvent::Removed { .. }) => {
                self.notifications.push("Friend removed".into(), ToastLevel::Info);
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }

            // ── Events with toasts (not forwarded to views) ──────────────
            SubscriptionEvent::Membership(MembershipEvent::JoinAccepted { .. }) => {
                self.notifications.push("Join request accepted!".into(), ToastLevel::Success);
                Some(Action::Render)
            }
            SubscriptionEvent::Membership(MembershipEvent::JoinRejected { ref reason, .. }) => {
                self.notifications.push(format!("Join rejected: {reason}"), ToastLevel::Warning);
                Some(Action::Render)
            }
            SubscriptionEvent::Membership(MembershipEvent::Joined { ref display_name, .. }) => {
                self.notifications.push(format!("{display_name} joined"), ToastLevel::Info);
                Some(Action::Render)
            }
            SubscriptionEvent::Membership(MembershipEvent::Left { ref pseudonym, .. }) => {
                self.notifications.push(crate::helpers::abbreviate_key(pseudonym).clone(), ToastLevel::Info);
                Some(Action::Render)
            }
            SubscriptionEvent::Membership(MembershipEvent::Kicked { ref target_pseudonym, .. }) => {
                self.notifications.push(format!("{} was kicked", crate::helpers::abbreviate_key(target_pseudonym)), ToastLevel::Warning);
                Some(Action::Render)
            }
            SubscriptionEvent::Membership(MembershipEvent::Banned { ref target_pseudonym, .. }) => {
                self.notifications.push(format!("{} was banned", crate::helpers::abbreviate_key(target_pseudonym)), ToastLevel::Warning);
                Some(Action::Render)
            }
            SubscriptionEvent::Friend(FriendEvent::Rejected { .. }) => {
                self.notifications.push("Friend request rejected".into(), ToastLevel::Warning);
                Some(Action::Render)
            }
            SubscriptionEvent::Crypto(CryptoEvent::AdminKeypairGranted { .. }) => {
                self.notifications.push("Admin keypair granted".into(), ToastLevel::Success);
                Some(Action::Render)
            }
            SubscriptionEvent::Governance(
                GovernanceEvent::ChannelsChanged { ref community }
                | GovernanceEvent::RolesChanged { ref community }
                | GovernanceEvent::MetadataChanged { ref community }
            ) => {
                self.load_dashboard_data();
                self.load_community_info(community);
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Social(SocialEvent::ThreadCreated { ref thread_name, .. }) => {
                self.notifications.push(format!("Thread created: {thread_name}"), ToastLevel::Info);
                Some(Action::Render)
            }
            SubscriptionEvent::Social(SocialEvent::EventCreated { ref title, .. }) => {
                self.notifications.push(format!("Event created: {title}"), ToastLevel::Info);
                Some(Action::Render)
            }
            SubscriptionEvent::Social(SocialEvent::EventReminder { ref title, minutes_until_start, .. }) => {
                self.notifications.push(format!("{title} starts in {minutes_until_start} min"), ToastLevel::Warning);
                Some(Action::Render)
            }
            SubscriptionEvent::Social(SocialEvent::GameServerAdded { ref label, .. }) => {
                self.notifications.push(format!("Game server added: {label}"), ToastLevel::Info);
                Some(Action::Render)
            }
            SubscriptionEvent::System(SystemEvent::Announcement { ref body, .. }) => {
                let preview = if body.len() > 80 { &body[..80] } else { body };
                self.notifications.push(format!("Announcement: {preview}"), ToastLevel::Info);
                Some(Action::Render)
            }
            SubscriptionEvent::System(SystemEvent::RaidAlert { active, .. }) => {
                let msg = if active { "Raid alert activated!" } else { "Raid alert cleared" };
                let level = if active { ToastLevel::Error } else { ToastLevel::Success };
                self.notifications.push(msg.into(), level);
                Some(Action::Render)
            }
            SubscriptionEvent::System(SystemEvent::ChannelLockdown { locked, .. }) => {
                let msg = if locked { "Channel locked down" } else { "Channel lockdown lifted" };
                let level = if locked { ToastLevel::Warning } else { ToastLevel::Success };
                self.notifications.push(msg.into(), level);
                Some(Action::Render)
            }
            SubscriptionEvent::System(SystemEvent::Kicked { ref community }) => {
                self.notifications.push(
                    format!("You were kicked from {}", crate::helpers::abbreviate_key(community)),
                    ToastLevel::Error,
                );
                Some(Action::Render)
            }

            // ── Silent events (no UI impact) ─────────────────────────────
            SubscriptionEvent::Network(
                NetworkEvent::WatchRenewed { .. } | NetworkEvent::WatchReestablished { .. }
            )
            | SubscriptionEvent::System(
                SystemEvent::BootstrapRequested { .. }
                | SystemEvent::BootstrapReceived { .. }
                | SystemEvent::SyncRequested { .. }
            ) => None,

            SubscriptionEvent::Network(NetworkEvent::WatchFailed { ref record_key, .. }) => {
                tracing::warn!(record_key, "watch failed");
                None
            }

            // ── All other events: render only (badges, status bar, etc.) ─
            _ => Some(Action::Render),
        }
    }

    /// Determine the current keymap context for hint display.
    pub(crate) fn current_context(&self) -> KeymapContext {
        if self.nav.overlay().is_some() || self.search.visible {
            KeymapContext::Overlay
        } else if self.nav.input_mode() {
            KeymapContext::Input
        } else {
            KeymapContext::Default
        }
    }

}
