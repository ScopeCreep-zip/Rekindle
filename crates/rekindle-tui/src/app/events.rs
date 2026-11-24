//! Event → Action translation for terminal, transport, and subscription events.

use super::super::action::{Action, ToastLevel};
use super::super::event::Event;
use super::super::keybinds::KeymapContext;
use super::super::navigator::KeyResolution;
use super::App;

use rekindle_types::subscription_events::{
    ChannelMessageEvent, FriendEvent, GovernanceEvent, MembershipEvent, NetworkEvent,
    NotificationEvent, SocialEvent, SubscriptionEvent, SystemEvent, VoiceEvent, VoiceScope,
};

/// How to name a call in a toast — `#channel`, or the peer for a DM.
fn call_label(scope: &VoiceScope) -> String {
    match scope {
        VoiceScope::Community { channel, .. } => format!("#{channel}"),
        VoiceScope::Dm { peer_key } => {
            format!("with {}", crate::helpers::abbreviate_key(peer_key))
        }
    }
}

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

                if self.intent.visible() {
                    return self.intent.handle_key(key);
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
                            mouse.column,
                            mouse.row,
                            self.nav.tab_bar_row,
                        ) {
                            self.nav.tab_bar.select(tab_idx);
                            self.transition_to_selected_tab();
                            return Some(Action::Render);
                        }
                        self.nav
                            .current_view_mut()
                            .handle_click(mouse.column, mouse.row)
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

    /// Our DM changed delivery state (plan E2.5): a failure is a toast;
    /// the state goes to the active view.
    fn dm_send_status(&mut self, event: SubscriptionEvent) -> Action {
        if let SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus {
            ref conversation_id,
            state: rekindle_types::message::SendState::Failed,
            ..
        }) = event
        {
            let short = crate::helpers::abbreviate_key(conversation_id);
            self.notifications
                .push(format!("A message to {short} failed"), ToastLevel::Error);
        }
        let _ = self
            .action_tx
            .send(Action::SubscriptionEvent(Box::new(event)));
        Action::Render
    }

    /// A DM arrived: a toast, the DMs tab's unread, and the active view.
    fn dm_received(&mut self, peer_key: &str, event: SubscriptionEvent) -> Action {
        let short = crate::helpers::abbreviate_key(peer_key);
        self.notifications
            .push(format!("New DM from {short}"), ToastLevel::Info);
        if self.nav.tab_bar.selected_id() != Some("dms") {
            self.nav.tab_bar.increment_unread("dms");
        }
        let _ = self
            .action_tx
            .send(Action::SubscriptionEvent(Box::new(event)));
        Action::Render
    }

    /// The status bar's network indicator takes the Host's bars and
    /// attachment.
    fn network_event(&mut self, event: &NetworkEvent) -> Action {
        match event {
            NetworkEvent::Strength { bars } => self.network_bars = *bars,
            NetworkEvent::AttachmentChanged { is_attached, .. } => {
                self.node_was_connected = *is_attached;
            }
            // Plan RV1: the pause and the deaths warning show in the
            // status line.
            NetworkEvent::RoutePaused { .. }
            | NetworkEvent::RouteResumed {}
            | NetworkEvent::RouteDeathsAboveBaseline { .. } => {
                self.route_alert.fold(event);
            }
            // Routed by the caller's other arms.
            NetworkEvent::LocalRoutesDied { .. }
            | NetworkEvent::RemoteRoutesDied { .. }
            | NetworkEvent::ValueChanged { .. }
            | NetworkEvent::OwnRecordsChanged { .. } => {}
        }
        Action::Render
    }

    /// A toast the user reads, then a redraw.
    fn toast(&mut self, text: String, level: ToastLevel) -> Action {
        self.notifications.push(text, level);
        Action::Render
    }

    /// Our community key went missing or behind, or arrived (KP): the
    /// Host's status, shown as it says.
    fn key_status_toast(&mut self, status: &rekindle_types::dto::community::KeyStatus) -> Action {
        let (text, level) = match status.state {
            rekindle_types::dto::community::KeyState::Pending => (
                "Waiting for the community key from a member who holds it".to_owned(),
                ToastLevel::Warning,
            ),
            rekindle_types::dto::community::KeyState::Current => {
                ("Community key received".to_owned(), ToastLevel::Info)
            }
        };
        self.notifications.push(text, level);
        self.load_dashboard_data();
        Action::Render
    }

    /// Convert a daemon subscription event into a TUI action.
    ///
    /// Every `SubscriptionEvent` variant is explicitly matched — no wildcard.
    /// Each event maps to the most appropriate TUI response: render for visual
    /// updates, toast for user-facing notifications, both for important events.
    pub(crate) fn subscription_event_to_action(
        &mut self,
        event: SubscriptionEvent,
    ) -> Option<Action> {
        self.idle_frames = 0;

        match event {
            // An action the backend began, waiting for this identity's
            // review (plan D3.4).
            SubscriptionEvent::IntentPending(review) => Some(Action::ShowIntent(Box::new(review))),
            // Our own presence (plan E1.2): the dashboard shows it live.
            SubscriptionEvent::Presence(
                rekindle_types::subscription_events::PresenceEvent::SelfChanged { snapshot, .. },
            ) => Some(Action::OwnPresence(Box::new(snapshot))),
            // The status bar's network indicator (plan E7.2): the Host's
            // smoothed bars and whether the node is attached; its route
            // warnings (plan RV1).
            SubscriptionEvent::Network(
                event @ (NetworkEvent::Strength { .. }
                | NetworkEvent::AttachmentChanged { .. }
                | NetworkEvent::RoutePaused { .. }
                | NetworkEvent::RouteResumed {}
                | NetworkEvent::RouteDeathsAboveBaseline { .. }),
            ) => Some(self.network_event(&event)),
            // ── Events forwarded to active view (real-time data push) ─────
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New { .. }) => {
                if self.nav.tab_bar.selected_id() != Some("communities") {
                    self.nav.tab_bar.increment_unread("communities");
                }
                let _ = self
                    .action_tx
                    .send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
                ref peer_key,
                ..
            }) => {
                let peer_key = peer_key.clone();
                Some(self.dm_received(&peer_key, event))
            }
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus { .. }) => {
                Some(self.dm_send_status(event))
            }
            SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::Edited { .. }
                | ChannelMessageEvent::Deleted { .. }
                // A gap in the peer's stream (plan E2.5): the DM view
                // draws its marker.
                | ChannelMessageEvent::DmHistoryGap { .. }
                // Disappearing timer notices and expiries (plan E7.1).
                | ChannelMessageEvent::DisappearingTimerChanged { .. }
                | ChannelMessageEvent::MessagesExpired { .. }
                // A channel outbox gap opened or closed (plan E3.3, D23).
                | ChannelMessageEvent::ChannelHistoryGap { .. }
                | ChannelMessageEvent::ChannelHistoryGapClosed { .. },
            )
            | SubscriptionEvent::Voice(
                VoiceEvent::ModeChanged { .. }
                | VoiceEvent::MuteChanged { .. }
                | VoiceEvent::DeafenChanged { .. }
                | VoiceEvent::RosterUpdated { .. }
                // Local-session events: forwarded and re-rendered like
                // the rest, but without a toast — quality and speaking
                // change often enough that toasting them would bury
                // everything else.
                | VoiceEvent::LocalJoined { .. }
                | VoiceEvent::SpeakingChanged { .. }
                | VoiceEvent::DeviceChanged { .. }
                | VoiceEvent::PacketsDropped { .. }
                | VoiceEvent::ConnectionQuality { .. },
            )
            | SubscriptionEvent::System(
                // New feed entries (plan E7.15): the open Activity view
                // adds them as the Host named them, without asking again.
                SystemEvent::ActivityAppended { .. }
                // The Servers view composed again (plan E7.2 G3): the open
                // Servers view shows it as the Host pushed it.
                | SystemEvent::ServersViewChanged { .. }
                // A subject's Infoview composed again (plan E7.15 IV7): the
                // open Infoview shows it as the Host pushed it.
                | SystemEvent::Infoview(..),
            ) => {
                let _ = self
                    .action_tx
                    .send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Voice(VoiceEvent::Joined {
                ref pseudonym,
                ref scope,
                ..
            }) => {
                let short = crate::helpers::abbreviate_key(pseudonym);
                self.notifications
                    .push(format!("{short} joined voice {}", call_label(scope)), ToastLevel::Info);
                let _ = self
                    .action_tx
                    .send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Voice(VoiceEvent::Left {
                ref pseudonym,
                ref scope,
            }) => {
                let short = crate::helpers::abbreviate_key(pseudonym);
                self.notifications
                    .push(format!("{short} left voice {}", call_label(scope)), ToastLevel::Info);
                let _ = self
                    .action_tx
                    .send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Friend(FriendEvent::RequestReceived {
                ref display_name, ..
            }) => {
                self.reload_friends();
                self.notifications.push(
                    format!("Friend request from {display_name}"),
                    ToastLevel::Info,
                );
                if self.nav.tab_bar.selected_id() != Some("friends") {
                    self.nav.tab_bar.increment_unread("friends");
                }
                let _ = self
                    .action_tx
                    .send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Friend(FriendEvent::Accepted { .. }) => {
                self.reload_friends();
                self.notifications
                    .push("Friend request accepted!".into(), ToastLevel::Success);
                let _ = self
                    .action_tx
                    .send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }
            SubscriptionEvent::Friend(FriendEvent::Removed { .. }) => {
                self.reload_friends();
                self.notifications
                    .push("Friend removed".into(), ToastLevel::Info);
                let _ = self
                    .action_tx
                    .send(Action::SubscriptionEvent(Box::new(event)));
                Some(Action::Render)
            }

            // ── Events with toasts (not forwarded to views) ──────────────
            SubscriptionEvent::Membership(MembershipEvent::Joined {
                ref pseudonym,
                ref display_name,
                ..
            }) => {
                let toast = format!("{} joined", member_label(pseudonym, display_name.as_deref()));
                Some(self.toast(toast, ToastLevel::Info))
            }
            SubscriptionEvent::Membership(MembershipEvent::Left { ref pseudonym, .. }) => {
                Some(self.toast(crate::helpers::abbreviate_key(pseudonym).clone(), ToastLevel::Info))
            }
            SubscriptionEvent::Membership(MembershipEvent::Kicked {
                ref community,
                ref target_pseudonym,
                ref removed_by_name,
                ref reason,
                ..
            }) => {
                let toast = self.removal_toast(
                    community,
                    target_pseudonym,
                    removed_by_name.as_deref(),
                    reason.as_deref(),
                    "kicked",
                );
                Some(self.toast(toast, ToastLevel::Warning))
            }
            SubscriptionEvent::Membership(MembershipEvent::Banned {
                ref community,
                ref target_pseudonym,
                ref removed_by_name,
                ref reason,
                ..
            }) => {
                let toast = self.removal_toast(
                    community,
                    target_pseudonym,
                    removed_by_name.as_deref(),
                    reason.as_deref(),
                    "banned",
                );
                Some(self.toast(toast, ToastLevel::Warning))
            }
            SubscriptionEvent::Friend(FriendEvent::Rejected { .. }) => {
                self.reload_friends();
                Some(self.toast("Friend request rejected".into(), ToastLevel::Warning))
            }
            // A permission change re-reads the community as a channel change
            // does (plan E3.2 S9): the dashboard shows what we may do.
            SubscriptionEvent::Governance(
                GovernanceEvent::ChannelsChanged { .. }
                | GovernanceEvent::EffectivePermissionsChanged { .. }
                | GovernanceEvent::ChannelPermissionsChanged { .. },
            ) => {
                self.load_dashboard_data();
                Some(Action::Render)
            }
            // Our community key went missing or behind, or arrived (KP):
            // the Host's status, shown as it says.
            SubscriptionEvent::Governance(GovernanceEvent::KeyStatus { ref status, .. }) => {
                Some(self.key_status_toast(status))
            }
            SubscriptionEvent::Social(SocialEvent::ThreadCreated { ref thread, .. }) => {
                Some(self.toast(format!("Thread created: {}", thread.name), ToastLevel::Info))
            }
            SubscriptionEvent::Social(SocialEvent::EventCreated { ref event, .. }) => {
                Some(self.toast(format!("Event created: {}", event.title), ToastLevel::Info))
            }
            SubscriptionEvent::Social(SocialEvent::EventReminder {
                ref title,
                minutes_until_start,
                ..
            }) => Some(self.toast(
                format!("{title} starts in {minutes_until_start} min"),
                ToastLevel::Warning,
            )),
            SubscriptionEvent::Social(SocialEvent::GameServerAdded { ref server, .. }) => {
                Some(self.toast(format!("Game server added: {}", server.label), ToastLevel::Info))
            }
            SubscriptionEvent::System(SystemEvent::SkinChanged { .. }) => Some(self.reskin(&event)),
            SubscriptionEvent::System(SystemEvent::QuickWipeChanged {
                armed,
                ref shortcut,
            }) => Some(self.quick_wipe_toast(armed, shortcut.as_deref())),

            // ── Silent events (no UI impact) ─────────────────────────────
            SubscriptionEvent::System(
                SystemEvent::BootstrapRequested { .. }
                | SystemEvent::BootstrapReceived { .. }
,
            ) => None,

            // ── Friend rows: re-read the list the Host projects ──────────
            SubscriptionEvent::Friend(FriendEvent::IdentityPinMismatch {
                ref display_name, ..
            }) => {
                self.reload_friends();
                Some(self.toast(
                    format!(
                        "{}'s keys are not signed by the identity you know. Nothing was \
                         sent; verify their safety number.",
                        crate::helpers::sanitize_for_display(display_name)
                    ),
                    ToastLevel::Error,
                ))
            }
            // The Host's app-level alerts (a refused memory lock, unsent
            // changes kept, a declined request): shown as the Host wrote them.
            SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
                ref title,
                ref body,
                ..
            }) => Some(self.toast(format!("{title}: {body}"), ToastLevel::Warning)),
            SubscriptionEvent::Friend(FriendEvent::HandshakeFailed { ref reason, .. }) => {
                self.reload_friends();
                Some(self.toast(
                    format!("Secure session not set up: {reason}"),
                    ToastLevel::Warning,
                ))
            }
            SubscriptionEvent::Friend(
                FriendEvent::HandshakeRetrying { .. }
                | FriendEvent::Reachability { .. }
                | FriendEvent::IdentityPinRestored { .. }
                | FriendEvent::BlockChanged { .. }
                | FriendEvent::GameInvite { .. }
                | FriendEvent::CreatorInvited { .. }
                | FriendEvent::CreatorInviteWithdrawn { .. }
                | FriendEvent::CoCreatorStatus { .. }
                | FriendEvent::CoCreationFinished { .. }
                | FriendEvent::GroupChanged { .. }
                | FriendEvent::NicknameChanged { .. }
                | FriendEvent::DisplayNameChanged { .. }
                | FriendEvent::BioChanged { .. }
                | FriendEvent::KeyChanged { .. }
                | FriendEvent::VerificationChanged { .. }
                | FriendEvent::Added { .. }
                | FriendEvent::RequestAcknowledged { .. }
                | FriendEvent::RemoveAcknowledged { .. }
                | FriendEvent::ProfileKeyRotated { .. },
            )
            // A friend's game changed: Join follows the Host's `joinable`
            // on the re-read rows (plan E7.15 IV3).
            | SubscriptionEvent::Presence(
                rekindle_types::subscription_events::PresenceEvent::FriendChanged {
                    snapshot:
                        rekindle_types::subscription_events::PresenceSnapshot { game: Some(_), .. },
                    ..
                },
            ) => {
                self.reload_friends();
                Some(Action::Render)
            }

            // ── Render only: badges, status bar, the open view ───────────
            SubscriptionEvent::UnreadChanged { .. }
            | SubscriptionEvent::ChannelMessage(_)
            | SubscriptionEvent::Typing(_)
            | SubscriptionEvent::Presence(_)
            | SubscriptionEvent::Membership(_)
            | SubscriptionEvent::Crypto(_)
            | SubscriptionEvent::Voice(_)
            | SubscriptionEvent::Governance(_)
            | SubscriptionEvent::Social(_)
            | SubscriptionEvent::Call(_)
            | SubscriptionEvent::GroupDm(_)
            | SubscriptionEvent::Notification(_)
            | SubscriptionEvent::Network(_)
            // A file transfer's state or progress (plan E7.2 G12): the
            // TUI has no Downloads view, so the open view re-renders.
            | SubscriptionEvent::Transfer(_)
            | SubscriptionEvent::System(_) => Some(Action::Render),
        }
    }

    /// The quick wipe was armed or disarmed (plan E7.1). The TUI has no
    /// global grab: the panic shortcut is the GUI's, so it is named as such.
    fn quick_wipe_toast(&mut self, armed: bool, shortcut: Option<&str>) -> Action {
        let msg = match (armed, shortcut) {
            (true, Some(chord)) => format!("Quick wipe armed (GUI panic shortcut {chord})"),
            (true, None) => "Quick wipe armed".to_owned(),
            (false, _) => "Quick wipe disarmed".to_owned(),
        };
        self.notifications.push(msg, ToastLevel::Warning);
        Action::Render
    }

    /// Re-read the friend list when it is on screen: its rows (pin alert,
    /// handshake state, group, nickname, key, block list) are the Host's.
    fn reload_friends(&self) {
        if matches!(
            self.nav.current_view(),
            super::super::views::ViewKind::FriendList
        ) {
            self.load_friend_list();
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

/// "Ann (abcd1234…ef01)", or the short pseudonym alone while the member's
/// card is not read (plan E3.2 F9-2): a name is always shown beside the
/// key, so two members with one name stay apart.
fn member_label(pseudonym: &str, name: Option<&str>) -> String {
    let key = crate::helpers::abbreviate_key(pseudonym);
    name.map_or_else(|| key.clone(), |name| format!("{name} ({key})"))
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
