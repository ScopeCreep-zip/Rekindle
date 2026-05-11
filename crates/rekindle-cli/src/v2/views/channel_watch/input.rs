//! Channel watch input handling — action processing, focused key dispatch, click.

use crossterm::event::KeyEvent;

use super::state::ChannelWatchView;
use crate::v2::tui::action::Action;
use crate::v2::tui::components::Component;
use crate::v2::tui::components::input_box::InputMode;
use crate::v2::tui::focus::FocusId;
use rekindle_types::display::DecryptedMessageDisplay;

pub fn handle_update(view: &mut ChannelWatchView, action: Action) -> Option<Action> {
    match action {
        Action::FocusNext => view.focus.next(),
        Action::FocusPrev => view.focus.prev(),
        Action::ToggleSidebar => { view.sidebar_visible = !view.sidebar_visible; view.update_focus_ring(); }
        Action::EnterInputMode => { view.focus.set(FocusId::InputBox); }
        Action::ExitInputMode => {
            view.focus.set(FocusId::MessageList);
            view.input_box.set_mode(InputMode::Compose);
        }
        Action::OpenSplitDm { peer_key } => {
            view.open_split_dm(&peer_key);
        }
        Action::CloseSplitDm => {
            view.split_dm.close();
            view.split_dm_message_list = None;
            view.split_dm_input_box = None;
            view.update_focus_ring();
        }
        Action::ReplyToSelected => {
            if let Some(idx) = view.message_list.selected_index() {
                if let Some(msg) = view.message_list.message_at(idx) {
                    view.input_box.set_mode(InputMode::Reply {
                        message_id: msg.message_id.clone(),
                        author: msg.author_display_name.clone(),
                    });
                    view.focus.set(FocusId::InputBox);
                }
            }
        }
        Action::EditSelected => {
            if let Some(idx) = view.message_list.selected_index() {
                if let Some(msg) = view.message_list.message_at(idx) {
                    view.input_box.set_mode(InputMode::Edit { message_id: msg.message_id.clone() });
                    view.focus.set(FocusId::InputBox);
                }
            }
        }
        Action::InputSubmit => {
            if view.focus.is_focused(FocusId::SplitDmInput) {
                if let Some(action) = view.handle_split_dm_submit() {
                    return Some(action);
                }
                return None;
            }

            let text = view.input_box.content();
            if !text.trim().is_empty() && !view.input_box.is_over_limit() {
                if text == "/patch" || text.starts_with("/patch ") {
                    let args = text.strip_prefix("/patch").unwrap_or("").trim();
                    let file_paths: Vec<&str> = if args.is_empty() {
                        Vec::new()
                    } else {
                        args.split_whitespace().collect()
                    };
                    let cwd = std::env::current_dir().unwrap_or_default();
                    match crate::v2::patch::generate::generate_patch(&cwd, &file_paths, false) {
                        Ok(patch) => {
                            if patch.diff.trim().is_empty() {
                                view.input_box.clear();
                                return Some(Action::ShowToast {
                                    message: "No changes to create a patch from".into(),
                                    level: crate::v2::tui::action::ToastLevel::Warning,
                                });
                            }
                            let desc = if args.is_empty() { String::new() }
                            else { format!("Patch for: {args}\n\n") };
                            let body = format!("{desc}```patch\n{}\n```", patch.diff);
                            let now = rekindle_utils::timestamp_ms();
                            view.message_list.push(pending_msg("you", &body, now));
                            view.input_box.clear();
                            return Some(Action::SendChannelMessage {
                                community: view.community.clone(),
                                channel: view.channel.clone(),
                                text: body,
                                reply_to: None,
                            });
                        }
                        Err(e) => {
                            view.input_box.clear();
                            return Some(Action::ShowToast {
                                message: format!("Patch generation failed: {e}"),
                                level: crate::v2::tui::action::ToastLevel::Error,
                            });
                        }
                    }
                }

                let action = match view.input_box.mode() {
                    InputMode::Edit { message_id } => {
                        view.message_list.update_body(message_id, &text);
                        Action::EditMessage {
                            community: view.community.clone(), channel: view.channel.clone(),
                            message_id: message_id.clone(), new_body: text,
                        }
                    }
                    InputMode::Reply { message_id, .. } => {
                        let now = rekindle_utils::timestamp_ms();
                        view.message_list.push(pending_msg("you", &text, now));
                        Action::SendChannelMessage {
                            community: view.community.clone(), channel: view.channel.clone(),
                            text, reply_to: Some(message_id.clone()),
                        }
                    }
                    InputMode::Compose => {
                        let now = rekindle_utils::timestamp_ms();
                        view.message_list.push(pending_msg("you", &text, now));
                        Action::SendChannelMessage {
                            community: view.community.clone(), channel: view.channel.clone(),
                            text, reply_to: None,
                        }
                    }
                };
                view.input_box.clear();
                return Some(action);
            }
        }
        Action::ScrollDown(n) => { for _ in 0..n { view.message_list.scroll_down(); } }
        Action::ScrollUp(n) => { for _ in 0..n { view.message_list.scroll_up(); } }
        Action::ScrollToBottom => view.message_list.scroll_to_bottom(),
        Action::ScrollToTop => view.message_list.scroll_to_top(),
        Action::ScrollPageDown => { for _ in 0..10 { view.message_list.scroll_down(); } }
        Action::ScrollPageUp => { for _ in 0..10 { view.message_list.scroll_up(); } }
        Action::Resize(w, _) => { view.terminal_width = w; view.update_focus_ring(); }
        Action::ScrollToMessage { ref message_id } => {
            view.message_list.scroll_to_message(message_id);
            view.focus.set(FocusId::MessageList);
        }
        Action::FileSelected { ref path } => {
            if view.focus.is_focused(FocusId::InputBox) {
                view.input_box.insert_text(path);
            } else if view.focus.is_focused(FocusId::SplitDmInput) {
                if let Some(ref mut ib) = view.split_dm_input_box {
                    ib.insert_text(path);
                }
            }
        }
        _ => {}
    }
    None
}

pub fn handle_focused_key(view: &mut ChannelWatchView, key: KeyEvent) -> Option<Action> {
    match view.focus.current() {
        FocusId::ChannelTree => view.channel_tree.handle_key(key),
        FocusId::MessageList => view.message_list.handle_key(key),
        FocusId::InputBox => {
            if key.code == crossterm::event::KeyCode::Char('p')
                && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            {
                return Some(Action::OpenQuickSwitcher);
            }
            if matches!(key.code, crossterm::event::KeyCode::Char(_)) && view.input_box.should_emit_typing() {
                let _ = view.input_box.handle_key(key);
                return Some(Action::SendChannelTyping {
                    community: view.community.clone(),
                    channel: view.channel.clone(),
                });
            }
            view.input_box.handle_key(key)
        }
        FocusId::PeerList => view.peer_list.handle_key(key),
        FocusId::SplitDmMessages => view.split_dm_message_list.as_mut().and_then(|ml| ml.handle_key(key)),
        FocusId::SplitDmInput => {
            if matches!(key.code, crossterm::event::KeyCode::Char(_)) {
                if let Some(ref mut ib) = view.split_dm_input_box {
                    if ib.should_emit_typing() {
                        let _ = ib.handle_key(key);
                        return Some(Action::SendDmTyping { peer_key: view.split_dm.peer_key.clone() });
                    }
                }
            }
            view.split_dm_input_box.as_mut().and_then(|ib| ib.handle_key(key))
        }
        _ => None,
    }
}

pub fn handle_click(view: &mut ChannelWatchView, column: u16, row: u16) -> Option<Action> {
    for (&id, rect) in &view.click_rects {
        if column >= rect.x && column < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height {
            view.focus.set(id);
            if id == FocusId::InputBox || id == FocusId::SplitDmInput {
                return Some(Action::EnterInputMode);
            }
            return None;
        }
    }
    None
}

fn pending_msg(author: &str, body: &str, timestamp: u64) -> DecryptedMessageDisplay {
    DecryptedMessageDisplay {
        message_id: format!("pending-{timestamp}"),
        sequence: 0,
        author_pseudonym: String::new(),
        author_display_name: author.to_string(),
        body: body.to_string(),
        timestamp,
        reply_to_sequence: None,
        mek_generation: 0,
        is_encrypted: false,
        needs_mek: None,
        delivery_status: rekindle_types::display::DeliveryStatus::Sending,
    }
}
