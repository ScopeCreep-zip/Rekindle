//! TEA reducer — process_action() maps Actions to state mutations.

use super::super::action::{Action, SearchMode, ToastLevel};
use super::super::terminal::Tui;
use super::App;
use crate::views::ViewKind;

impl App {
    /// Process a single action — the TEA reducer.
    pub(crate) fn process_action(&mut self, action: Action, tui: &mut Tui) -> anyhow::Result<()> {
        match action {
            Action::Render => {
                let skip = if self.loading_spinner.is_active() {
                    false
                } else if self.idle_frames > 30 {
                    !self.idle_frames.is_multiple_of(4)
                } else if self.idle_frames > 4 {
                    !self.idle_frames.is_multiple_of(2)
                } else {
                    false
                };
                self.idle_frames = self.idle_frames.saturating_add(1);
                if !skip {
                    tui.draw(|frame| self.draw(frame))?;
                }
            }
            Action::Tick => {
                self.notifications.tick();
                self.loading_spinner.tick();
                self.nav.current_view_mut().tick()?;

                if let Some(deadline) = self.clipboard_clear_at {
                    if std::time::Instant::now() >= deadline {
                        if let Some(ref mut cb) = self.clipboard {
                            let _ = cb.set_text("");
                        }
                        self.clipboard_clear_at = None;
                        self.notifications
                            .push("Clipboard auto-cleared".into(), ToastLevel::Info);
                    }
                }
            }
            Action::Quit => {
                let use_unicode = self.theme.use_unicode();
                if self.nav.quit(use_unicode) {
                    self.should_quit = true;
                } else {
                    self.load_dashboard_data();
                }
            }
            Action::Back => {
                let use_unicode = self.theme.use_unicode();
                self.nav.back(use_unicode);
            }
            Action::Resize(w, h) => {
                let _ = self.nav.current_view_mut().update(Action::Resize(w, h));
                tui.draw(|frame| self.draw(frame))?;
            }
            Action::FocusNext => self.nav.current_view_mut().focus_ring().next(),
            Action::FocusPrev => self.nav.current_view_mut().focus_ring().prev(),
            action @ (Action::EnterInputMode | Action::ReplyToSelected | Action::EditSelected) => {
                self.nav.enter_input_mode();
                let _ = self.nav.current_view_mut().update(action);
            }
            Action::ExitInputMode => {
                self.nav.exit_input_mode();
                self.nav
                    .current_view_mut()
                    .focus_ring()
                    .set(crate::tui::focus::FocusId::MessageList);
            }
            Action::Cancel => {
                if self.nav.overlay().is_some() {
                    self.nav.close_overlay();
                } else if self.search.visible {
                    self.search.close();
                } else if self.nav.input_mode() {
                    self.nav.exit_input_mode();
                } else if !self.notifications.is_empty() {
                    // Warning/Error toasts are sticky (never auto-dismiss); Esc
                    // clears them oldest-first when nothing else owns the key.
                    self.notifications.dismiss_oldest();
                }
            }
            Action::ToggleHelp => self.nav.toggle_help(),
            Action::Refresh => {
                let _ = self.nav.current_view_mut().update(Action::Refresh);
            }
            Action::ToggleSidebar => {
                self.nav.toggle_sidebar();
                let _ = self.nav.current_view_mut().update(Action::ToggleSidebar);
            }
            Action::OpenSearch(mode) => {
                let items = self.build_search_items(mode);
                self.search.open(mode, items);
            }
            Action::OpenQuickSwitcher => {
                let items = self.build_search_items(SearchMode::QuickSwitch);
                self.search.open(SearchMode::QuickSwitch, items);
            }
            Action::NextTab => {
                self.nav.tab_bar.next();
                self.transition_to_selected_tab();
            }
            Action::PrevTab => {
                self.nav.tab_bar.prev();
                self.transition_to_selected_tab();
            }

            // View transitions
            Action::ShowDashboard => {
                self.nav
                    .navigate(ViewKind::Dashboard, self.theme.use_unicode());
                self.load_dashboard_data();
            }
            Action::ShowIdentitySettings => {
                self.nav
                    .navigate(ViewKind::IdentitySettings, self.theme.use_unicode());
                self.load_dashboard_data(); // StatusSnapshot has identity + route data
            }
            Action::ShowChannel { community, channel } => {
                let kind = ViewKind::ChannelWatch {
                    community: community.clone(),
                    channel: channel.clone(),
                };
                self.nav.navigate(kind, self.theme.use_unicode());
                self.load_channel_history(&community, &channel);
            }
            Action::ShowDmInbox => {
                self.nav
                    .navigate(ViewKind::DmInbox, self.theme.use_unicode());
                self.load_dm_inbox();
            }
            Action::ShowDmThread { peer_key } => {
                self.nav
                    .navigate(ViewKind::DmThread { peer_key }, self.theme.use_unicode());
            }
            Action::ShowFriendList => {
                self.nav
                    .navigate(ViewKind::FriendList, self.theme.use_unicode());
                self.load_friend_list();
            }
            Action::ShowVoiceSession { community, channel } => {
                self.nav.navigate(
                    ViewKind::VoiceSession { community, channel },
                    self.theme.use_unicode(),
                );
            }
            Action::ShowDoctor => {
                self.nav
                    .navigate(ViewKind::Doctor, self.theme.use_unicode());
                self.load_dashboard_data(); // StatusSnapshot includes checks
            }
            Action::ShowCommunityInfo { community } => {
                let kind = ViewKind::CommunityInfo {
                    community: community.clone(),
                };
                self.nav.navigate(kind, self.theme.use_unicode());
                self.load_community_info(&community);
            }

            // Overlays
            Action::CloseOverlay => {
                self.nav.close_overlay();
                self.search.close();
            }
            Action::ConfirmOverlay => {
                if self.confirm.is_confirmed() {
                    if let Some(deferred) = self.pending_confirm_action.take() {
                        self.confirm.hide();
                        let _ = self.action_tx.send(deferred);
                    } else {
                        self.confirm.hide();
                    }
                } else {
                    self.confirm.hide();
                    self.pending_confirm_action = None;
                }
            }
            Action::LeaveVoice => self.process_leave_voice(),
            Action::ToggleMute => {
                let _ = self.nav.current_view_mut().update(Action::ToggleMute);
            }
            Action::ToggleDeafen => {
                let _ = self.nav.current_view_mut().update(Action::ToggleDeafen);
            }

            // Friend operations
            Action::AcceptFriendRequest(id) => self.spawn_accept_friend(id),
            Action::RejectFriendRequest(id) => self.spawn_reject_friend(id),
            Action::RemoveFriend { ref peer_key } => self.process_remove_friend(peer_key),
            Action::LeaveCommunity { ref community } => self.process_leave_community(community),

            // Clipboard
            Action::YankToClipboard { ref text } => self.yank_to_clipboard(text),

            Action::ShowToast { message, level } => self.notifications.push(message, level),
            Action::CommandComplete(result) => self.process_command_complete(*result)?,
            Action::CommandFailed { context, error } => {
                self.notifications
                    .push(format!("{context}: {error}"), ToastLevel::Error);
            }
            Action::SendChannelMessage {
                community,
                channel,
                text,
                reply_to,
            } => {
                self.spawn_send_channel_message(&community, &channel, text, reply_to);
            }
            Action::SendDm { peer_key, text } => {
                self.spawn_send_dm(peer_key, text);
            }

            Action::SubscriptionEvent(ref event) => {
                self.nav.current_view_mut().on_subscription_event(event)?;
            }

            action => {
                if let Some(chained) = self.nav.current_view_mut().update(action)? {
                    let _ = self.action_tx.send(chained);
                }
            }
        }
        Ok(())
    }

    /// Handle a completed async command: update local caches, then forward
    /// the result to the active view.
    fn process_command_complete(
        &mut self,
        result: super::super::action::CommandResult,
    ) -> anyhow::Result<()> {
        use super::super::action::CommandResult;
        self.loading_spinner.stop();
        // Extract identity and community caches before forwarding to view
        match &result {
            CommandResult::IdentityLoaded {
                public_key,
                display_name,
            } => {
                self.cached_identity = Some(super::CachedIdentity {
                    public_key: public_key.clone(),
                    display_name: display_name.clone(),
                });
                self.nav
                    .dashboard_mut()
                    .set_identity(public_key, display_name);
            }
            CommandResult::CommunityListLoaded { communities } => {
                self.cached_communities = communities
                    .iter()
                    .map(|c| super::CachedCommunity {
                        governance_key: c.governance_key.clone(),
                        name: c.name.clone(),
                    })
                    .collect();
            }
            _ => {}
        }
        self.nav.current_view_mut().on_command_result(result)
    }

    /// Copy text to the system clipboard, lazily initializing the clipboard
    /// handle and scheduling a 30-second auto-clear.
    fn yank_to_clipboard(&mut self, text: &str) {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => self.clipboard = Some(cb),
                Err(e) => {
                    self.notifications
                        .push(format!("Clipboard unavailable: {e}"), ToastLevel::Warning);
                    return;
                }
            }
        }
        let cb = self.clipboard.as_mut().expect("initialized above");
        match cb.set_text(text) {
            Ok(()) => {
                self.notifications.push(
                    "Copied to clipboard (auto-clear in 30s)".into(),
                    ToastLevel::Info,
                );
                self.clipboard_clear_at =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
            }
            Err(e) => self
                .notifications
                .push(format!("Clipboard write failed: {e}"), ToastLevel::Warning),
        }
    }

    /// Two-step leave-voice flow: first press arms a confirmation, second
    /// press (after confirm) performs the disconnect.
    fn process_leave_voice(&mut self) {
        if self.pending_confirm_action.is_some() {
            self.nav
                .navigate(ViewKind::Dashboard, self.theme.use_unicode());
            self.notifications
                .push("Left voice channel".into(), ToastLevel::Info);
            self.pending_confirm_action = None;
        } else {
            self.pending_confirm_action = Some(Action::LeaveVoice);
            self.confirm
                .show("Leave voice channel?", "You will be disconnected.");
        }
    }

    /// Two-step remove-friend flow guarded by a confirmation dialog.
    fn process_remove_friend(&mut self, peer_key: &str) {
        if self.pending_confirm_action.is_some() {
            self.notifications.push(
                format!(
                    "Removed friend {}",
                    crate::helpers::abbreviate_key(peer_key)
                ),
                ToastLevel::Info,
            );
            self.pending_confirm_action = None;
        } else {
            self.pending_confirm_action = Some(Action::RemoveFriend {
                peer_key: peer_key.to_string(),
            });
            self.confirm.show(
                format!("Remove {}?", crate::helpers::abbreviate_key(peer_key)),
                "They will no longer see your messages or presence.",
            );
        }
    }

    /// Two-step leave-community flow guarded by a confirmation dialog.
    fn process_leave_community(&mut self, community: &str) {
        if self.pending_confirm_action.is_some() {
            let name = self.community_name(community).to_string();
            self.notifications
                .push(format!("Left '{name}'"), ToastLevel::Info);
            self.pending_confirm_action = None;
            self.nav
                .navigate(ViewKind::Dashboard, self.theme.use_unicode());
        } else {
            let name = self.community_name(community).to_string();
            self.pending_confirm_action = Some(Action::LeaveCommunity {
                community: community.to_string(),
            });
            self.confirm.show(
                format!("Leave '{name}'?"),
                "You will lose access to all channels.",
            );
        }
    }
}
