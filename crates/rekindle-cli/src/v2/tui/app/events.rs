//! Event → Action translation for terminal and subscription events.

use super::super::action::{Action, ToastLevel};
use super::super::event::Event;
use super::super::keybinds::KeymapContext;
use super::super::navigator::KeyResolution;
use super::App;
use crate::v2::helpers;

use rekindle_types::subscription_events::{
    SubscriptionEvent, ChannelMessageEvent, VoiceEvent, NetworkEvent,
    FriendEvent, MembershipEvent, CryptoEvent, GovernanceEvent, SocialEvent, SystemEvent,
};

impl App {
    /// Convert a terminal event into an Action.
    pub(crate) fn event_to_action(&mut self, event: Event) -> Option<Action> {
        match event {
            Event::Tick => Some(Action::Tick),
            Event::Render => Some(Action::Render),
            Event::Resize(w, h) => Some(Action::Resize(w, h)),
            Event::Key(key) => {
                self.idle_frames = 0;
                // File content search overlay (Ctrl+G)
                if self.file_content_search.visible {
                    let result = self.file_content_search.handle_key(key);
                    // Re-run content search on each keystroke
                    if matches!(key.code, crossterm::event::KeyCode::Char(_) | crossterm::event::KeyCode::Backspace)
                        && self.file_content_search.visible
                        && !self.file_content_search.query.is_empty()
                    {
                        if let Some(ref mut search) = self.search {
                            let matches = search.grep(
                                &self.file_content_search.query,
                                fff_search::GrepMode::PlainText,
                                0, 50,
                            );
                            let items: Vec<super::super::components::file_content_search::ContentMatch> = matches.iter().map(|(path, line_num, content)| {
                                super::super::components::file_content_search::ContentMatch {
                                    file_path: path.clone(),
                                    line_number: *line_num,
                                    line_content: content.clone(),
                                    is_definition: false, // simplified — full classification available via GrepSearchOptions
                                }
                            }).collect();
                            let total = items.len();
                            self.file_content_search.set_results(items, total, total);
                        }
                    }
                    return result;
                }
                if self.search_overlay.visible {
                    let result = self.search_overlay.handle_key(key);
                    // Live fff re-query: when user types in QuickSwitch mode,
                    // re-populate file results based on the current query.
                    // Debounced to 50ms to prevent input lag on large projects.
                    let debounce_ok = self.last_search_query_at
                        .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(50));
                    if matches!(key.code, crossterm::event::KeyCode::Char(_) | crossterm::event::KeyCode::Backspace)
                        && self.search_overlay.visible
                        && self.search_overlay.mode == super::super::action::SearchMode::QuickSwitch
                        && debounce_ok
                    {
                        self.last_search_query_at = Some(std::time::Instant::now());
                        if let Some(ref search) = self.search {
                            let query = &self.search_overlay.query;
                            if !query.is_empty() {
                                // Remove old file items and add fresh fff results
                                self.search_overlay.items.retain(|item| item.detail != "file");
                                for (path, _score) in search.search_files(query, None, 30) {
                                    self.search_overlay.items.push(
                                        super::super::components::search_overlay::SearchItem {
                                            label: path.clone(),
                                            detail: "file".into(),
                                            action: super::super::action::Action::FileSelected { path },
                                        }
                                    );
                                }
                                // Re-filter with the new items
                                let query_clone = query.clone();
                                self.search_overlay.filtered_indices = crate::v2::tui::components::search_overlay::filter::filter_items(
                                    &self.search_overlay.items, &query_clone,
                                );
                                if self.search_overlay.filtered_indices.is_empty() {
                                    self.search_overlay.list_state.select(None);
                                } else {
                                    self.search_overlay.list_state.select(Some(0));
                                }
                            }
                        }
                    }
                    return result;
                }
                if self.confirm.visible {
                    use crossterm::event::KeyCode;
                    if matches!(key.code, KeyCode::Tab | KeyCode::Left | KeyCode::Right) {
                        self.confirm.toggle_focus();
                        return None;
                    }
                }
                let use_unicode = self.theme.use_unicode();
                match self.nav.handle_key(key, &self.keymap, use_unicode) {
                    KeyResolution::Action(action) => Some(action),
                    KeyResolution::ForwardToView(key) => self.nav.current_view_mut().handle_focused_key(key),
                    KeyResolution::Consumed => None,
                    KeyResolution::ForceQuit => { self.should_quit = true; Some(Action::Render) }
                }
            }
            Event::Mouse(mouse) => {
                self.idle_frames = 0;
                use crossterm::event::MouseEventKind;
                match mouse.kind {
                    MouseEventKind::ScrollDown => Some(Action::ScrollDown(3)),
                    MouseEventKind::ScrollUp => Some(Action::ScrollUp(3)),
                    MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                        if let Some(tab_idx) = self.nav.tab_bar.click_tab(mouse.column, mouse.row, self.nav.tab_bar_row) {
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
                    // Insert pasted text into the active input box character by character.
                    // ratatui-textarea handles insertion via KeyEvent simulation.
                    for ch in text.chars() {
                        let key = crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Char(ch),
                            crossterm::event::KeyModifiers::NONE,
                        );
                        self.nav.current_view_mut().handle_focused_key(key);
                    }
                    Some(Action::Render)
                } else {
                    None
                }
            }
            Event::Init | Event::FocusGained | Event::FocusLost => None,
        }
    }

    /// Convert a daemon subscription event into a TUI action.
    /// Exhaustive match — every variant handled explicitly.
    pub(crate) fn subscription_event_to_action(&mut self, event: SubscriptionEvent) -> Option<Action> {
        self.idle_frames = 0;
        match event {
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New { .. }) => {
                if self.nav.tab_bar.selected_id() != Some("communities") { self.nav.tab_bar.increment_unread("communities"); }
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived { ref peer_key, is_self: false, .. }) => {
                let short = helpers::abbreviate_key(peer_key);
                self.notifications.push(format!("New DM from {short}"), ToastLevel::Info);
                if self.nav.tab_bar.selected_id() != Some("dms") { self.nav.tab_bar.increment_unread("dms"); }
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived { .. } | ChannelMessageEvent::Edited { .. } | ChannelMessageEvent::Deleted { .. })
            | SubscriptionEvent::Voice(VoiceEvent::ModeChanged { .. } | VoiceEvent::MuteChanged { .. } | VoiceEvent::DeafenChanged { .. } | VoiceEvent::RosterUpdated { .. })
            | SubscriptionEvent::Network(NetworkEvent::AttachmentChanged { .. })
            | SubscriptionEvent::Presence(_) | SubscriptionEvent::Typing(_) | SubscriptionEvent::UnreadChanged { .. } => {
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Voice(VoiceEvent::Joined { ref pseudonym, ref channel, .. }) => {
                self.notifications.push(format!("{} joined voice #{channel}", helpers::abbreviate_key(pseudonym)), ToastLevel::Info);
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Voice(VoiceEvent::Left { ref pseudonym, ref channel, .. }) => {
                self.notifications.push(format!("{} left voice #{channel}", helpers::abbreviate_key(pseudonym)), ToastLevel::Info);
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Friend(FriendEvent::RequestReceived { ref display_name, .. }) => {
                self.notifications.push(format!("Friend request from {display_name}"), ToastLevel::Info);
                if self.nav.tab_bar.selected_id() != Some("friends") { self.nav.tab_bar.increment_unread("friends"); }
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
            SubscriptionEvent::Membership(MembershipEvent::Kicked { ref target_pseudonym, .. }) => {
                self.notifications.push(format!("{} was kicked", helpers::abbreviate_key(target_pseudonym)), ToastLevel::Warning);
                Some(Action::Render)
            }
            SubscriptionEvent::Membership(MembershipEvent::Banned { ref target_pseudonym, .. }) => {
                self.notifications.push(format!("{} was banned", helpers::abbreviate_key(target_pseudonym)), ToastLevel::Warning);
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
            SubscriptionEvent::Governance(GovernanceEvent::ChannelsChanged { ref community } | GovernanceEvent::RolesChanged { ref community } | GovernanceEvent::MetadataChanged { ref community }) => {
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
            SubscriptionEvent::Social(SocialEvent::EventReminder { ref community, ref event_id, ref title, minutes_until_start, .. }) => {
                use super::super::components::notification_rail::{RailSignal, SignalScope, SignalPriority};
                self.notifications.push(format!("{title} starts in {minutes_until_start} min"), ToastLevel::Warning);
                // Persistent community rail signal for imminent events
                self.rails.set(RailSignal {
                    id: format!("event:{community}:{event_id}"),
                    scope: SignalScope::Community,
                    text: format!("📅 {title} — starts in {minutes_until_start} min"),
                    priority: if minutes_until_start <= 5 { SignalPriority::Warning } else { SignalPriority::Info },
                    dismissible: true,
                });
                Some(Action::Render)
            }
            SubscriptionEvent::System(SystemEvent::Announcement { ref community, ref body, .. }) => {
                use super::super::components::notification_rail::{RailSignal, SignalScope, SignalPriority};
                let preview = body.char_indices().nth(80).map_or(body.as_str(), |(i, _)| &body[..i]);
                self.notifications.push(format!("Announcement: {preview}"), ToastLevel::Info);
                // Persistent community rail signal for announcements
                if let Some(ref gov) = community {
                    self.rails.set(RailSignal {
                        id: format!("announce:{gov}"),
                        scope: SignalScope::Community,
                        text: format!("📢 {preview}"),
                        priority: SignalPriority::Info,
                        dismissible: true,
                    });
                }
                Some(Action::Render)
            }
            SubscriptionEvent::System(SystemEvent::RaidAlert { ref community, active, .. }) => {
                use super::super::components::notification_rail::{RailSignal, SignalScope, SignalPriority};
                let (msg, level) = if active { ("Raid alert activated!", ToastLevel::Error) } else { ("Raid alert cleared", ToastLevel::Success) };
                self.notifications.push(msg.into(), level);
                if active {
                    self.rails.set(RailSignal {
                        id: format!("raid:{community}"),
                        scope: SignalScope::Community,
                        text: format!("RAID ALERT — {}", helpers::abbreviate_key(community)),
                        priority: SignalPriority::Critical,
                        dismissible: false,
                    });
                } else {
                    self.rails.remove(&format!("raid:{community}"));
                }
                Some(Action::Render)
            }
            SubscriptionEvent::System(SystemEvent::ChannelLockdown { ref community, locked, .. }) => {
                use super::super::components::notification_rail::{RailSignal, SignalScope, SignalPriority};
                let (msg, level) = if locked { ("Channel locked down", ToastLevel::Warning) } else { ("Lockdown lifted", ToastLevel::Success) };
                self.notifications.push(msg.into(), level);
                if locked {
                    self.rails.set(RailSignal {
                        id: format!("lockdown:{community}"),
                        scope: SignalScope::Channel,
                        text: "🔒 Channel locked — non-operators cannot send".into(),
                        priority: SignalPriority::Warning,
                        dismissible: false,
                    });
                } else {
                    self.rails.remove(&format!("lockdown:{community}"));
                }
                Some(Action::Render)
            }
            SubscriptionEvent::System(SystemEvent::Kicked { ref community }) => {
                self.notifications.push(format!("Kicked from {}", helpers::abbreviate_key(community)), ToastLevel::Error);
                // Remove from cached communities so the tab doesn't show a dead entry
                self.cached_communities.retain(|c| c.governance_key != *community);
                // Forward to views so channel_watch/community_info can clean up state
                let _ = self.action_tx.send(Action::SubscriptionEvent(Box::new(event)));
                // Navigate away from the community view
                Some(Action::ShowDashboard)
            }
            // Silent events
            SubscriptionEvent::Network(NetworkEvent::WatchRenewed { .. } | NetworkEvent::WatchReestablished { .. })
            | SubscriptionEvent::System(SystemEvent::BootstrapRequested { .. } | SystemEvent::BootstrapReceived { .. } | SystemEvent::SyncRequested { .. }) => None,
            SubscriptionEvent::Network(NetworkEvent::WatchFailed { ref record_key, .. }) => {
                tracing::warn!(record_key, "watch failed");
                None
            }
            // All other events: render only
            _ => Some(Action::Render),
        }
    }

    pub(crate) fn current_context(&self) -> KeymapContext {
        if self.nav.overlay().is_some() || self.search_overlay.visible || self.file_content_search.visible {
            KeymapContext::Overlay
        } else if self.nav.input_mode() {
            KeymapContext::Input
        } else {
            KeymapContext::Default
        }
    }
}
