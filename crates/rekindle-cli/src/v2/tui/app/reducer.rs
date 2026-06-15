//! TEA reducer — process_action maps Actions to state mutations.

use std::sync::Arc;

use crate::v2::prelude::{ChatRequest, DaemonRequest, ReadContext};

use super::super::action::{Action, CommandResult, SearchMode, ToastLevel};
use super::super::terminal::Tui;
use super::App;
use crate::v2::helpers;
use crate::v2::views::ViewKind;

impl App {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn process_action(&mut self, action: Action, tui: &mut Tui) -> anyhow::Result<()> {
        match action {
            Action::Render => {
                let skip = if self.loading_spinner.is_active() { false }
                else if self.idle_frames > 30 { !self.idle_frames.is_multiple_of(4) }
                else if self.idle_frames > 4 { !self.idle_frames.is_multiple_of(2) }
                else { false };
                self.idle_frames = self.idle_frames.saturating_add(1);
                if !skip { tui.draw(|frame| self.draw(frame))?; }
            }
            Action::Tick => {
                self.notifications.tick();
                self.loading_spinner.tick();
                if let Ok(Some(a)) = self.nav.current_view_mut().tick() {
                    let _ = self.action_tx.send(a);
                }
                if let Some(deadline) = self.clipboard_clear_at {
                    if std::time::Instant::now() >= deadline {
                        if let Some(ref mut cb) = self.clipboard { let _ = cb.set_text(""); }
                        self.clipboard_clear_at = None;
                        self.notifications.push("Clipboard auto-cleared".into(), ToastLevel::Info);
                    }
                }
            }
            Action::Quit => {
                let use_unicode = self.theme.use_unicode();
                if self.nav.quit(use_unicode) { self.should_quit = true; }
                else { self.load_dashboard_data(); }
            }
            Action::Back => { self.nav.back(self.theme.use_unicode()); }
            Action::Resize(w, h) => {
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::Resize(w, h)) {
                    let _ = self.action_tx.send(a);
                }
                tui.draw(|frame| self.draw(frame))?;
            }
            Action::FocusNext => self.nav.current_view_mut().focus_ring().next(),
            Action::FocusPrev => self.nav.current_view_mut().focus_ring().prev(),
            action @ (Action::EnterInputMode | Action::ReplyToSelected | Action::EditSelected) => {
                self.nav.enter_input_mode();
                if let Ok(Some(a)) = self.nav.current_view_mut().update(action) {
                    let _ = self.action_tx.send(a);
                }
            }
            Action::ExitInputMode => {
                self.nav.exit_input_mode();
                self.nav.current_view_mut().focus_ring().set(crate::v2::tui::focus::FocusId::MessageList);
            }
            Action::Cancel => {
                if self.nav.overlay().is_some() { self.nav.close_overlay(); }
                else if self.search_overlay.visible { self.search_overlay.close(); }
                else if self.file_content_search.visible { self.file_content_search.close(); }
                else if self.nav.input_mode() { self.nav.exit_input_mode(); }
                else { self.rails.dismiss_first_dismissible(); }
            }
            Action::ToggleHelp => self.nav.toggle_help(),
            Action::Refresh => {
                // If the current view is a channel watch with pins panel visible, load pins
                if let ViewKind::ChannelWatch { ref community, .. } = *self.nav.current_view_kind() {
                    self.load_pins(community);
                }
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::Refresh) {
                    let _ = self.action_tx.send(a);
                }
                // Trigger fff file tree rescan (picks up new/deleted files since last scan)
                if let Some(ref search) = self.search {
                    if let Err(e) = search.rescan() {
                        tracing::debug!(error = %e, "fff rescan failed");
                    }
                }
            }
            Action::ToggleSidebar => {
                self.nav.toggle_sidebar();
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::ToggleSidebar) {
                    let _ = self.action_tx.send(a);
                }
            }
            Action::OpenSearch(mode) => {
                let items = self.build_search_items(mode);
                self.search_overlay.open(mode, items);
            }
            Action::OpenQuickSwitcher => {
                let items = self.build_search_items(SearchMode::QuickSwitch);
                self.search_overlay.open(SearchMode::QuickSwitch, items);
            }
            Action::OpenFileContentSearch => {
                self.file_content_search.open();
            }
            Action::NextTab => { self.nav.tab_bar.next(); self.transition_to_selected_tab(); }
            Action::PrevTab => { self.nav.tab_bar.prev(); self.transition_to_selected_tab(); }

            // ── View transitions ─────────────────────────────
            Action::ShowDashboard => {
                self.rails.clear_scope(super::super::components::notification_rail::SignalScope::Community);
                self.rails.clear_scope(super::super::components::notification_rail::SignalScope::Channel);
                self.nav.navigate(ViewKind::Dashboard, self.theme.use_unicode());
                self.load_dashboard_data();
                let client = Arc::clone(&self.client);
                tokio::spawn(async move { let _ = client.subscribe_all().await; });
            }
            Action::ShowIdentitySettings => {
                self.nav.navigate(ViewKind::IdentitySettings, self.theme.use_unicode());
                self.load_dashboard_data();
            }
            Action::ShowChannel { community, channel } => {
                let kind = ViewKind::ChannelWatch { community: community.clone(), channel: channel.clone() };
                self.nav.navigate(kind, self.theme.use_unicode());
                self.load_channel_history(&community, &channel);
                self.load_community_info(&community);
                let client = Arc::clone(&self.client);
                let gov = community.clone();
                let ch = channel.clone();
                tokio::spawn(async move {
                    let _ = client.subscribe_scoped(&gov).await;
                    let _ = client.request_ok(DaemonRequest::Chat(ChatRequest::MarkRead {
                        context: ReadContext::Channel { community: gov, channel: ch },
                    })).await;
                });
            }
            Action::ShowDmInbox => {
                self.nav.navigate(ViewKind::DmInbox, self.theme.use_unicode());
                self.load_dm_inbox();
            }
            Action::ShowDmThread { peer_key } => {
                self.nav.navigate(ViewKind::DmThread { peer_key: peer_key.clone() }, self.theme.use_unicode());
                self.load_dm_thread(&peer_key);
                let client = Arc::clone(&self.client);
                let pk = peer_key;
                tokio::spawn(async move {
                    let _ = client.request_ok(DaemonRequest::Chat(ChatRequest::MarkRead {
                        context: ReadContext::Dm { peer: pk },
                    })).await;
                });
            }
            Action::ShowFriendList => {
                self.nav.navigate(ViewKind::FriendList, self.theme.use_unicode());
                self.load_friend_list();
            }
            Action::ShowVoiceSession { community, channel } | Action::JoinVoice { community, channel } => {
                self.nav.navigate(ViewKind::VoiceSession { community, channel }, self.theme.use_unicode());
            }
            Action::ShowDoctor => {
                self.nav.navigate(ViewKind::Doctor, self.theme.use_unicode());
                self.load_dashboard_data();
            }
            Action::ShowCommunityInfo { community } => {
                self.nav.navigate(ViewKind::CommunityInfo { community: community.clone() }, self.theme.use_unicode());
                self.load_community_info(&community);
            }
            Action::OpenThread { ref thread_id, ref thread_name } => {
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::OpenThread {
                    thread_id: thread_id.clone(), thread_name: thread_name.clone(),
                }) {
                    let _ = self.action_tx.send(a);
                }
                self.load_thread_messages(thread_id);
            }
            Action::CloseThread => {
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::CloseThread) {
                    let _ = self.action_tx.send(a);
                }
            }
            Action::ShowModeration { community } => {
                self.nav.navigate(ViewKind::Moderation { community: community.clone() }, self.theme.use_unicode());
                self.load_moderation_data(&community);
            }
            Action::ShowInvites { community } => {
                self.nav.navigate(ViewKind::Invite { community: community.clone() }, self.theme.use_unicode());
                self.load_invites(&community);
            }
            Action::ShowEvents { community } => {
                self.nav.navigate(ViewKind::Events { community: community.clone() }, self.theme.use_unicode());
                self.load_events(&community);
            }
            Action::ShowOnboarding { community } => {
                self.nav.navigate(ViewKind::Onboarding { community: community.clone() }, self.theme.use_unicode());
                self.load_onboarding(&community);
            }

            // ── Split pane DM ────────────────────────────────
            Action::OpenSplitDm { peer_key } => {
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::OpenSplitDm { peer_key: peer_key.clone() }) {
                    let _ = self.action_tx.send(a);
                }
                self.load_dm_thread(&peer_key);
            }
            Action::CloseSplitDm => {
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::CloseSplitDm) {
                    let _ = self.action_tx.send(a);
                }
            }

            // ── Overlays ─────────────────────────────────────
            Action::OpenOverlay(kind) => self.nav.open_overlay(kind),
            Action::CloseOverlay => {
                self.nav.close_overlay();
                self.search_overlay.close();
                self.file_content_search.close();
            }
            Action::ConfirmOverlay => {
                if self.confirm.is_confirmed() {
                    if let Some(deferred) = self.pending_confirm_action.take() {
                        self.confirm.hide();
                        let _ = self.action_tx.send(deferred);
                    } else { self.confirm.hide(); }
                } else { self.confirm.hide(); self.pending_confirm_action = None; }
            }

            // ── Voice ────────────────────────────────────────
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
            Action::ToggleMute => {
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::ToggleMute) {
                    let _ = self.action_tx.send(a);
                }
            }
            Action::ToggleDeafen => {
                if let Ok(Some(a)) = self.nav.current_view_mut().update(Action::ToggleDeafen) {
                    let _ = self.action_tx.send(a);
                }
            }

            // ── Friend operations ────────────────────────────
            Action::AcceptFriendRequest(id) => self.spawn_accept_friend(id),
            Action::RejectFriendRequest(id) => self.spawn_reject_friend(id),
            Action::RemoveFriend { ref peer_key } => {
                if self.pending_confirm_action.is_some() {
                    self.notifications.push(format!("Removed {}", helpers::abbreviate_key(peer_key)), ToastLevel::Info);
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(Action::RemoveFriend { peer_key: peer_key.clone() });
                    self.confirm.show(format!("Remove {}?", helpers::abbreviate_key(peer_key)), "They will no longer see your messages.");
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

            // ── Reaction + Pin operations ────────────────
            Action::AddReaction { ref community, ref channel, ref message_id, ref emoji } => {
                self.spawn_add_reaction(community.clone(), channel.clone(), message_id.clone(), emoji.clone());
            }
            Action::RemoveReaction { ref community, ref channel, ref message_id, ref emoji } => {
                self.spawn_remove_reaction(community.clone(), channel.clone(), message_id.clone(), emoji.clone());
            }
            Action::UnpinMessage { ref community, ref channel, ref message_id } => {
                if self.pending_confirm_action.is_some() {
                    self.spawn_unpin_message(community.clone(), channel.clone(), message_id.clone());
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(action.clone());
                    self.confirm.show("Unpin this message?", "The message will no longer appear in the pins panel.");
                }
            }

            // ── Moderation operations ────────────────────
            Action::KickMember { ref community, ref pseudonym, ref display_name } => {
                if self.pending_confirm_action.is_some() {
                    self.spawn_kick_member(community.clone(), pseudonym.clone());
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(action.clone());
                    self.confirm.show(format!("Kick {display_name}?"), "They will be removed from the community but can rejoin.");
                }
            }
            Action::BanMember { ref community, ref pseudonym, ref display_name } => {
                if self.pending_confirm_action.is_some() {
                    self.spawn_ban_member(community.clone(), pseudonym.clone());
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(action.clone());
                    self.confirm.show(format!("Ban {display_name}?"), "They will be permanently banned and all channel keys rekeyed.");
                }
            }
            Action::UnbanMember { ref community, ref pseudonym } => {
                self.spawn_unban_member(community.clone(), pseudonym.clone());
            }
            Action::TimeoutMember { ref community, ref pseudonym, ref display_name, duration_secs } => {
                if self.pending_confirm_action.is_some() {
                    self.spawn_timeout_member(community.clone(), pseudonym.clone(), duration_secs);
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(action.clone());
                    self.confirm.show(format!("Timeout {display_name} for {duration_secs}s?"), "They will be unable to send messages for the duration.");
                }
            }
            Action::ApproveMember { ref community, ref pseudonym, ref display_name } => {
                if self.pending_confirm_action.is_some() {
                    self.spawn_approve_member(community.clone(), pseudonym.clone());
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(action.clone());
                    self.confirm.show(format!("Approve {display_name}?"), "They will gain full community access and receive channel MEKs.");
                }
            }
            Action::RejectMember { ref community, ref pseudonym, ref display_name } => {
                if self.pending_confirm_action.is_some() {
                    self.spawn_reject_member(community.clone(), pseudonym.clone());
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(action.clone());
                    self.confirm.show(format!("Reject {display_name}?"), "Their join request will be removed.");
                }
            }
            Action::CreateInvite { ref community, max_uses, ref expires_seconds } => {
                self.spawn_create_invite(community.clone(), max_uses, *expires_seconds);
            }
            Action::RevokeInvite { ref community, ref invite_code } => {
                if self.pending_confirm_action.is_some() {
                    self.spawn_revoke_invite(community.clone(), invite_code.clone());
                    self.pending_confirm_action = None;
                } else {
                    self.pending_confirm_action = Some(action.clone());
                    self.confirm.show(format!("Revoke invite {}?", helpers::abbreviate_key(invite_code)), "The invite code will no longer work.");
                }
            }

            // ── Clipboard ────────────────────────────────────
            Action::YankToClipboard { ref text } => {
                if self.clipboard.is_none() {
                    match arboard::Clipboard::new() {
                        Ok(cb) => self.clipboard = Some(cb),
                        Err(e) => { self.notifications.push(format!("Clipboard unavailable: {e}"), ToastLevel::Warning); return Ok(()); }
                    }
                }
                match self.clipboard.as_mut().expect("initialized").set_text(text) {
                    Ok(()) => {
                        self.notifications.push("Copied (auto-clear 30s)".into(), ToastLevel::Info);
                        self.clipboard_clear_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
                    }
                    Err(e) => self.notifications.push(format!("Clipboard failed: {e}"), ToastLevel::Warning),
                }
            }

            // ── Messages ─────────────────────────────────────
            Action::SendChannelMessage { community, channel, text, reply_to } => {
                self.spawn_send_channel_message(&community, &channel, text, reply_to);
            }
            Action::SendDm { peer_key, text } => { self.spawn_send_dm(peer_key, text); }
            Action::EditMessage { community, channel, message_id, new_body } => {
                self.spawn_edit_message(community, channel, message_id, new_body);
            }
            Action::DeleteMessage { community, channel, message_id } => {
                self.spawn_delete_message(community, channel, message_id);
            }
            Action::SendChannelTyping { community, channel } => {
                self.spawn_channel_typing(community, channel);
            }
            Action::SendDmTyping { peer_key } => {
                self.spawn_dm_typing(peer_key);
            }
            Action::LoadDmThread { peer_key } => {
                self.load_dm_thread(&peer_key);
            }

            // ── Patch operations ──────────────────────────────
            Action::ApplyPatch { ref message_id } => {
                // Two-phase: first invocation shows confirmation, second (after confirm) applies.
                if self.pending_confirm_action.is_some() {
                    // User confirmed — extract and apply
                    let index = self.nav.current_view().message_search_index();
                    let body = index.iter()
                        .find(|(id, _, _)| id == message_id)
                        .map(|(_, _, body)| body.clone());
                    if let Some(body) = body {
                        if let Some(diff_text) = crate::v2::patch::render::extract_patch_fence(&body) {
                            let patch = crate::v2::patch::render::parse_diff_to_patch(&diff_text);
                            let cwd = std::env::current_dir().unwrap_or_default();
                            let result = crate::v2::patch::apply::apply_patch(&cwd, &patch);
                            if let Some(ref error) = result.error {
                                self.notifications.push(format!("Patch failed: {error}"), ToastLevel::Error);
                            } else {
                                let files = result.applied_files.join(", ");
                                self.notifications.push(format!("Patch applied: {files}"), ToastLevel::Success);
                                if !result.clean_apply {
                                    self.notifications.push(
                                        "Warning: patch was against a different commit".into(),
                                        ToastLevel::Warning,
                                    );
                                }
                            }
                        } else {
                            self.notifications.push("No patch found in message".into(), ToastLevel::Warning);
                        }
                    } else {
                        self.notifications.push("Message no longer in buffer".into(), ToastLevel::Warning);
                    }
                    self.pending_confirm_action = None;
                } else {
                    // First invocation — show confirmation with affected file paths
                    let index = self.nav.current_view().message_search_index();
                    let body = index.iter()
                        .find(|(id, _, _)| id == message_id)
                        .map(|(_, _, body)| body.clone());
                    if let Some(body) = body {
                        if let Some(diff_text) = crate::v2::patch::render::extract_patch_fence(&body) {
                            let patch = crate::v2::patch::render::parse_diff_to_patch(&diff_text);
                            let file_list = patch.files.iter()
                                .map(|f| format!("  {} {}", f.status, f.path))
                                .collect::<Vec<_>>()
                                .join("\n");
                            self.pending_confirm_action = Some(Action::ApplyPatch { message_id: message_id.clone() });
                            self.confirm.show(
                                format!("Apply patch? ({})", patch.summary()),
                                format!("This will modify your local working tree:\n{file_list}"),
                            );
                        } else {
                            self.notifications.push("No patch found in message".into(), ToastLevel::Warning);
                        }
                    } else {
                        self.notifications.push("Message not found".into(), ToastLevel::Warning);
                    }
                }
            }
            Action::CopyPatch { ref message_id } => {
                let index = self.nav.current_view().message_search_index();
                let body = index.iter()
                    .find(|(id, _, _)| id == message_id)
                    .map(|(_, _, body)| body.clone());
                if let Some(body) = body {
                    if let Some(diff_text) = crate::v2::patch::render::extract_patch_fence(&body) {
                        let _ = self.action_tx.send(Action::YankToClipboard { text: diff_text });
                    } else {
                        self.notifications.push("No patch found in message".into(), ToastLevel::Warning);
                    }
                }
            }
            Action::DismissPatch { .. } => {
                // Toggle collapsed state on the patch — delegated to the view
                if let Ok(Some(chained)) = self.nav.current_view_mut().update(action) {
                    let _ = self.action_tx.send(chained);
                }
            }

            // ── File selection ────────────────────────────────
            Action::FileSelected { ref path } => {
                // Track file open for frecency ranking — future quick switcher
                // results will rank this file higher. The query comes from the
                // search overlay that produced this selection.
                let query = self.search_overlay.query.clone();
                if let Some(ref mut search) = self.search {
                    if let Some(base) = search.base_path() {
                        let abs_path = base.join(path).to_string_lossy().to_string();
                        search.on_open(&query, &abs_path);
                    }
                }

                if self.nav.input_mode() {
                    // Insert as inline code reference into the active input box
                    let formatted = format!("`{path}`");
                    if let Ok(Some(chained)) = self.nav.current_view_mut().update(
                        Action::FileSelected { path: formatted }
                    ) {
                        let _ = self.action_tx.send(chained);
                    }
                } else {
                    // Not in input mode — open file preview
                    let _ = self.action_tx.send(Action::ShowFilePreview {
                        path: path.clone(), line: None,
                    });
                }
            }
            Action::ShowFilePreview { path, line } => {
                self.nav.navigate(
                    ViewKind::FilePreview { path, line },
                    self.theme.use_unicode(),
                );
            }

            // ── Misc ─────────────────────────────────────────
            Action::RequestMek { community, channel } => {
                self.notifications.push(format!("MEK requested for #{channel}"), ToastLevel::Info);
                let tx = self.action_tx.clone();
                let client = Arc::clone(&self.client);
                let community_clone = community.clone();
                let channel_clone = channel.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.request_ok(DaemonRequest::Chat(ChatRequest::MekRequest {
                        community: community_clone,
                        channel: channel_clone,
                        generation: 0,
                    })).await {
                        let _ = tx.send(Action::CommandFailed {
                            context: "MEK request".into(),
                            error: e.to_string(),
                        });
                    }
                });
            }
            Action::SetPresence { status, message } => {
                let msg = message.as_deref().unwrap_or("");
                self.notifications.push(
                    format!("Status: {status}{}", if msg.is_empty() { String::new() } else { format!(" — {msg}") }),
                    ToastLevel::Success,
                );
            }
            Action::ShowToast { message, level } => self.notifications.push(message, level),
            Action::DismissToast => self.notifications.dismiss_oldest(),

            // ── Async results ────────────────────────────────
            Action::CommandComplete(mut result) => {
                self.loading_spinner.stop();
                match &mut *result {
                    CommandResult::IdentityLoaded { public_key, display_name, .. } => {
                        self.cached_identity = Some(super::CachedIdentity { public_key: public_key.clone(), display_name: display_name.clone() });
                        self.nav.dashboard_mut().set_identity(public_key, display_name);
                    }
                    CommandResult::CommunityListLoaded { communities } => {
                        self.cached_communities = communities.iter().map(|c| super::CachedCommunity {
                            governance_key: c.governance_key.clone(), name: c.name.clone(),
                        }).collect();
                        // Execute deferred session restore — only if the saved community still exists
                        if let Some((community, channel)) = self.pending_session_restore.take() {
                            let still_member = self.cached_communities.iter().any(|c| c.governance_key == community);
                            if still_member {
                                let action = match channel {
                                    Some(ch) => Action::ShowChannel { community, channel: ch },
                                    None => Action::ShowCommunityInfo { community },
                                };
                                let _ = self.action_tx.send(action);
                            } else {
                                tracing::info!(
                                    community = %helpers::abbreviate_key(&community),
                                    "session restore skipped — no longer a member"
                                );
                            }
                        }
                    }
                    CommandResult::StatusLoaded { ref mut snapshot } => {
                        self.node_was_connected = snapshot.is_attached;
                        self.cached_peer_count = snapshot.peer_count;

                        // Append local CLI-side checks (fff search, storage, version)
                        let storage_info = crate::v2::helpers::storage_dir(None)
                            .map_or("unknown".into(), |p| {
                                let size = crate::v2::helpers::dir_size(&p);
                                format!("{} ({})", p.display(), crate::v2::helpers::format_bytes(size))
                            });
                        snapshot.checks.push(rekindle_types::display::Check::pass(
                            "local.storage", "local", storage_info,
                        ));
                        snapshot.checks.push(rekindle_types::display::Check::pass(
                            "local.cli_version", "local", env!("CARGO_PKG_VERSION"),
                        ));
                        if let Some(ref search) = self.search {
                            let scanning = if search.is_scanning() { " (scanning...)" } else { "" };
                            snapshot.checks.push(rekindle_types::display::Check::pass(
                                "local.search_engine", "local",
                                format!("fff initialized{scanning}"),
                            ));
                        } else {
                            snapshot.checks.push(rekindle_types::display::Check::warn(
                                "local.search_engine", "local", "fff not initialized",
                            ).with_description("project-wide file search unavailable"));
                        }
                    }
                    _ => {}
                }
                if let Ok(Some(chained)) = self.nav.current_view_mut().on_command_result(*result) {
                    let _ = self.action_tx.send(chained);
                }
            }
            Action::CommandFailed { context, error } => {
                self.notifications.push(format!("{context}: {error}"), ToastLevel::Error);
            }

            // ── Subscription events ──────────────────────────
            Action::SubscriptionEvent(ref event) => {
                use rekindle_types::subscription_events::*;
                match event.as_ref() {
                    SubscriptionEvent::Network(NetworkEvent::AttachmentChanged { is_attached, .. }) => {
                        self.node_was_connected = *is_attached;
                    }
                    SubscriptionEvent::System(SystemEvent::IdentityCreated { public_key }) => {
                        self.notifications.push(
                            format!("Identity created: {}…", &public_key[..12.min(public_key.len())]),
                            ToastLevel::Success,
                        );
                    }
                    SubscriptionEvent::System(SystemEvent::IdentityRotated { new_public_key }) => {
                        self.notifications.push(
                            format!("Identity rotated: {}…", &new_public_key[..12.min(new_public_key.len())]),
                            ToastLevel::Info,
                        );
                    }
                    SubscriptionEvent::Membership(MembershipEvent::Created { community, .. }) => {
                        self.notifications.push(format!("Community created: {community}"), ToastLevel::Success);
                    }
                    SubscriptionEvent::Membership(MembershipEvent::CommunityJoined { community, .. }) => {
                        self.notifications.push(format!("Joined community: {community}"), ToastLevel::Success);
                    }
                    SubscriptionEvent::Membership(MembershipEvent::CommunityLeft { community, .. }) => {
                        self.notifications.push(format!("Left community: {community}"), ToastLevel::Info);
                    }
                    SubscriptionEvent::Friend(FriendEvent::RequestSent { target_profile_key, .. }) => {
                        self.notifications.push(
                            format!("Friend request sent to {}…", &target_profile_key[..12.min(target_profile_key.len())]),
                            ToastLevel::Success,
                        );
                    }
                    SubscriptionEvent::Friend(FriendEvent::Accepted { peer_key, .. }) => {
                        self.notifications.push(
                            format!("Friend accepted: {}…", &peer_key[..12.min(peer_key.len())]),
                            ToastLevel::Success,
                        );
                    }
                    SubscriptionEvent::Friend(FriendEvent::RequestReceived { display_name, .. }) => {
                        self.notifications.push(format!("Friend request from {display_name}"), ToastLevel::Info);
                    }
                    // All other events: no reducer-level state mutation needed.
                    // The primary event handler (events.rs) already produced toasts
                    // and forwarded to views. The reducer only handles state mutations
                    // that affect App fields (node_was_connected, cached_identity, etc).
                    SubscriptionEvent::Friend(
                        FriendEvent::Rejected { .. }
                        | FriendEvent::Removed { .. }
                        | FriendEvent::RequestAcknowledged { .. }
                        | FriendEvent::RemoveAcknowledged { .. }
                        | FriendEvent::ProfileKeyRotated { .. }
                    )
                    | SubscriptionEvent::Membership(_)
                    | SubscriptionEvent::Crypto(_)
                    | SubscriptionEvent::Voice(_)
                    | SubscriptionEvent::Governance(_)
                    | SubscriptionEvent::Social(_)
                    | SubscriptionEvent::Network(
                        NetworkEvent::LocalRoutesDied { .. }
                        | NetworkEvent::RemoteRoutesDied { .. }
                        | NetworkEvent::WatchRenewed { .. }
                        | NetworkEvent::WatchReestablished { .. }
                        | NetworkEvent::WatchFailed { .. }
                        | NetworkEvent::ValueChanged { .. }
                    )
                    | SubscriptionEvent::System(_)
                    | SubscriptionEvent::ChannelMessage(_)
                    | SubscriptionEvent::Typing(_)
                    | SubscriptionEvent::Presence(_)
                    | SubscriptionEvent::Dm(_)
                    | SubscriptionEvent::UnreadChanged { .. }
                    | SubscriptionEvent::BulkTransferProgress { .. } => {}
                }
                for a in self.nav.forward_event_to_all_views(event) {
                    let _ = self.action_tx.send(a);
                }
            }

            // ── Fallthrough ──────────────────────────────────
            action => {
                if let Ok(Some(chained)) = self.nav.current_view_mut().update(action) {
                    let _ = self.action_tx.send(chained);
                }
            }
        }
        Ok(())
    }
}
