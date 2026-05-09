//! TEA reducer — process_action() maps Actions to state mutations.

use rekindle_node::ipc::protocol::IpcRequest;

use super::super::action::{Action, SearchMode, ToastLevel};
use super::super::terminal::Tui;
use super::App;
use crate::views::ViewKind;

impl App {
    /// Process a single action — the TEA reducer.
    #[allow(clippy::too_many_lines)]
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
                        self.notifications.push("Clipboard auto-cleared".into(), ToastLevel::Info);
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
                self.nav.current_view_mut().focus_ring()
                    .set(crate::tui::focus::FocusId::MessageList);
            }
            Action::Cancel => {
                if self.nav.overlay().is_some() {
                    self.nav.close_overlay();
                } else if self.search.visible {
                    self.search.close();
                } else if self.nav.input_mode() {
                    self.nav.exit_input_mode();
                }
            }
            Action::ToggleHelp => self.nav.toggle_help(),
            Action::Refresh => { let _ = self.nav.current_view_mut().update(Action::Refresh); }
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
                self.nav.navigate(ViewKind::Dashboard, self.theme.use_unicode());
                self.load_dashboard_data();
                // Widen subscription back to all events
                let client = std::sync::Arc::clone(&self.client);
                tokio::spawn(async move {
                    if let Err(e) = client.subscribe_all().await {
                        tracing::warn!(error = %e, "failed to widen subscription");
                    }
                });
            }
            Action::ShowIdentitySettings => {
                self.nav.navigate(ViewKind::IdentitySettings, self.theme.use_unicode());
                self.load_dashboard_data(); // StatusSnapshot has identity + route data
            }
            Action::ShowChannel { community, channel } => {
                let kind = ViewKind::ChannelWatch { community: community.clone(), channel: channel.clone() };
                self.nav.navigate(kind, self.theme.use_unicode());
                self.load_channel_history(&community, &channel);
                self.load_community_info(&community);
                // Narrow subscription + mark-read on the daemon
                let client = std::sync::Arc::clone(&self.client);
                let gov = community.clone();
                let ch = channel.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.subscribe_scoped(&gov).await {
                        tracing::warn!(community = %gov, error = %e, "failed to scope subscription");
                    }
                    if let Err(e) = client.request_ok(IpcRequest::MarkRead {
                        context: rekindle_node::ipc::protocol::ReadContext::Channel { community: gov, channel: ch },
                    }).await {
                        tracing::debug!(error = %e, "mark-read failed");
                    }
                });
            }
            Action::ShowDmInbox => {
                self.nav.navigate(ViewKind::DmInbox, self.theme.use_unicode());
                self.load_dm_inbox();
            }
            Action::ShowDmThread { peer_key } => {
                self.nav.navigate(ViewKind::DmThread { peer_key: peer_key.clone() }, self.theme.use_unicode());
                self.load_dm_thread(&peer_key);
                // Mark DM as read on the daemon
                let client = std::sync::Arc::clone(&self.client);
                let pk = peer_key;
                tokio::spawn(async move {
                    if let Err(e) = client.request_ok(IpcRequest::MarkRead {
                        context: rekindle_node::ipc::protocol::ReadContext::Dm { peer: pk },
                    }).await {
                        tracing::debug!(error = %e, "DM mark-read failed");
                    }
                });
            }
            Action::ShowFriendList => {
                self.nav.navigate(ViewKind::FriendList, self.theme.use_unicode());
                self.load_friend_list();
            }
            Action::ShowVoiceSession { community, channel }
            | Action::JoinVoice { community, channel } => {
                self.nav.navigate(ViewKind::VoiceSession { community, channel }, self.theme.use_unicode());
            }
            Action::ShowDoctor => {
                self.nav.navigate(ViewKind::Doctor, self.theme.use_unicode());
                self.load_dashboard_data(); // StatusSnapshot includes checks
            }
            Action::ShowCommunityInfo { community } => {
                let kind = ViewKind::CommunityInfo { community: community.clone() };
                self.nav.navigate(kind, self.theme.use_unicode());
                self.load_community_info(&community);
            }

            // Overlays
            Action::OpenOverlay(kind) => self.nav.open_overlay(kind),
            Action::CloseOverlay => { self.nav.close_overlay(); self.search.close(); }
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
            Action::LeaveVoice => {
                if self.pending_confirm_action.is_some() {
                    self.nav.navigate(ViewKind::Dashboard, self.theme.use_unicode());
                    self.notifications.push("Left voice channel".into(), ToastLevel::Info);
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(Action::LeaveVoice);
                    self.confirm.show("Leave voice channel?", "You will be disconnected.");
                }
            }
            Action::ToggleMute => { let _ = self.nav.current_view_mut().update(Action::ToggleMute); }
            Action::ToggleDeafen => { let _ = self.nav.current_view_mut().update(Action::ToggleDeafen); }

            // Friend operations
            Action::AcceptFriendRequest(id) => self.spawn_accept_friend(id),
            Action::RejectFriendRequest(id) => self.spawn_reject_friend(id),
            Action::RemoveFriend { ref peer_key } => {
                if self.pending_confirm_action.is_some() {
                    self.notifications.push(
                        format!("Removed friend {}", crate::helpers::abbreviate_key(peer_key)),
                        ToastLevel::Info,
                    );
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(Action::RemoveFriend { peer_key: peer_key.clone() });
                    self.confirm.show(
                        format!("Remove {}?", crate::helpers::abbreviate_key(peer_key)),
                        "They will no longer see your messages or presence.",
                    );
                }
            }
            Action::LeaveCommunity { ref community } => {
                if self.pending_confirm_action.is_some() {
                    let name = self.community_name(community).to_string();
                    self.notifications.push(format!("Left '{name}'"), ToastLevel::Info);
                    self.pending_confirm_action = None;
                    self.nav.navigate(ViewKind::Dashboard, self.theme.use_unicode());
                } else {
                    let name = self.community_name(community).to_string();
                    self.pending_confirm_action = Some(Action::LeaveCommunity { community: community.clone() });
                    self.confirm.show(format!("Leave '{name}'?"), "You will lose access to all channels.");
                }
            }
            Action::RequestMek { community, channel } => {
                self.notifications.push(
                    format!("MEK requested for #{channel} in {}", crate::helpers::abbreviate_key(&community)),
                    ToastLevel::Info,
                );
            }

            // Clipboard
            Action::YankToClipboard { ref text } => {
                if self.clipboard.is_none() {
                    match arboard::Clipboard::new() {
                        Ok(cb) => self.clipboard = Some(cb),
                        Err(e) => {
                            self.notifications.push(format!("Clipboard unavailable: {e}"), ToastLevel::Warning);
                            return Ok(());
                        }
                    }
                }
                let cb = self.clipboard.as_mut().expect("initialized above");
                match cb.set_text(text) {
                    Ok(()) => {
                        self.notifications.push("Copied to clipboard (auto-clear in 30s)".into(), ToastLevel::Info);
                        self.clipboard_clear_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
                    }
                    Err(e) => self.notifications.push(format!("Clipboard write failed: {e}"), ToastLevel::Warning),
                }
            }

            Action::SetPresence { status, message } => {
                let msg = message.as_deref().unwrap_or("");
                self.notifications.push(
                    format!("Status set to {status}{}", if msg.is_empty() { String::new() } else { format!(" — {msg}") }),
                    ToastLevel::Success,
                );
            }
            Action::ShowToast { message, level } => self.notifications.push(message, level),
            Action::DismissToast => self.notifications.dismiss_oldest(),
            Action::CommandComplete(result) => {
                self.loading_spinner.stop();
                // Extract identity and community caches before forwarding to view
                match &*result {
                    super::super::action::CommandResult::IdentityLoaded { public_key, display_name } => {
                        self.cached_identity = Some(super::CachedIdentity {
                            public_key: public_key.clone(),
                            display_name: display_name.clone(),
                        });
                        self.nav.dashboard_mut().set_identity(public_key, display_name);
                    }
                    super::super::action::CommandResult::CommunityListLoaded { communities } => {
                        self.cached_communities = communities.iter().map(|c| super::CachedCommunity {
                            governance_key: c.governance_key.clone(),
                            name: c.name.clone(),
                        }).collect();
                    }
                    super::super::action::CommandResult::StatusLoaded { snapshot } => {
                        self.node_was_connected = snapshot.is_attached;
                        self.cached_peer_count = snapshot.peer_count;
                    }
                    _ => {}
                }
                self.nav.current_view_mut().on_command_result(*result)?;
            }
            Action::CommandFailed { context, error } => {
                self.notifications.push(format!("{context}: {error}"), ToastLevel::Error);
            }
            Action::SendChannelMessage { community, channel, text, reply_to } => {
                self.spawn_send_channel_message(&community, &channel, text, reply_to);
            }
            Action::SendDm { peer_key, text } => {
                self.spawn_send_dm(peer_key, text);
            }
            Action::EditMessage { community, channel, message_id, new_body } => {
                self.spawn_edit_message(community, channel, message_id, new_body);
            }
            Action::DeleteMessage { community, channel, message_id } => {
                self.spawn_delete_message(community, channel, message_id);
            }

            Action::SubscriptionEvent(ref event) => {
                // Update App-level cached state from specific events
                if let rekindle_types::subscription_events::SubscriptionEvent::Network(
                    rekindle_types::subscription_events::NetworkEvent::AttachmentChanged { is_attached, .. }
                ) = event.as_ref() {
                    self.node_was_connected = *is_attached;
                }
                self.nav.forward_event_to_all_views(event)?;
            }

            action => {
                if let Some(chained) = self.nav.current_view_mut().update(action)? {
                    let _ = self.action_tx.send(chained);
                }
            }
        }
        Ok(())
    }
}
