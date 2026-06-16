//! Channel watch input — channel tree, messages, compose, split DM, reactions,
//! reply/edit modes, /patch command, pins, threads, scroll, retry.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::v2::tui::components::input_box::{InputBox, InputBoxResult, InputMode};
use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::channels::ChannelKey;
use crate::v2::tui::state::dm::DmMessage;
use crate::v2::tui::state::emoji_picker::EmojiPickerState;
use crate::v2::tui::state::ephemeral::ToastLevel;
use crate::v2::tui::state::in_flight::{PendingSend, RequestKind};
use crate::v2::tui::state::confirm::PendingConfirmAction;
use crate::v2::tui::state::navigation::{InputContext, OverlayState, ViewKind};
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    community: &str,
    channel: &str,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    let TerminalEvent::Key(key) = event else {
        return vec![];
    };

    match state.nav.focus_ring.current() {
        FocusId::ChannelTree => handle_channel_tree_key(key, state, community, channel, caches),
        FocusId::MessageList => handle_message_key(key, state, community, channel, caches),
        FocusId::InputBox => handle_input_key(key, state, community, channel),
        FocusId::PeerList => handle_peer_key(key, state, community, channel),
        FocusId::SplitDmMessages => handle_split_dm_messages_key(key, state, community, channel, caches),
        FocusId::SplitDmInput => handle_split_dm_input(key, state, community, channel),
        FocusId::ThreadPanel => handle_thread_panel_key(key, state, community, channel),
        _ => vec![],
    }
}

fn ch_key(community: &str, channel: &str) -> ChannelKey {
    ChannelKey { community: community.to_string(), channel: channel.to_string() }
}

fn handle_channel_tree_key(
    key: &KeyEvent,
    state: &mut TuiState,
    community: &str,
    channel: &str,
    _caches: &mut RenderCaches,
) -> Vec<Effect> {
    let ck = ch_key(community, channel);
    match key.code {
        KeyCode::Char('h') => {
            state.nav.pop_view();
            vec![]
        }
        KeyCode::Char('l') => {
            // Focus right: channel tree → message list
            state.nav.focus_ring.set(FocusId::MessageList);
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                let max = state.communities.details.get(community)
                    .map_or(0, |d| d.channels.len().saturating_sub(1));
                let current = ch.channel_tree_selected.unwrap_or(0);
                ch.channel_tree_selected = Some((current + 1).min(max));
            }
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                let current = ch.channel_tree_selected.unwrap_or(0);
                ch.channel_tree_selected = Some(current.saturating_sub(1));
            }
            vec![]
        }
        KeyCode::Enter => {
            let selected = state.channels.channels.get(&ck)
                .and_then(|ch| ch.channel_tree_selected);
            if let Some(idx) = selected {
                if let Some(detail) = state.communities.details.get(community) {
                    if let Some(target_ch) = detail.channels.get(idx) {
                        return vec![Effect::Navigate(ViewKind::ChannelWatch {
                            community: community.to_string(),
                            channel: target_ch.name.clone(),
                        })];
                    }
                }
            }
            vec![]
        }
        KeyCode::Tab => { state.nav.focus_ring.next(); vec![] }
        _ => vec![],
    }
}

fn handle_message_key(
    key: &KeyEvent,
    state: &mut TuiState,
    community: &str,
    channel: &str,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    let ck = ch_key(community, channel);
    match key.code {
        KeyCode::Char('h') => {
            // Focus left: message list → channel tree (if sidebar visible)
            state.nav.focus_ring.set(FocusId::ChannelTree);
            vec![]
        }
        KeyCode::Char('l') => {
            // Focus right: message list → peer list (if visible)
            state.nav.focus_ring.set(FocusId::PeerList);
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            caches.channel.entry(ck).or_default().list_state.select_next();
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            caches.channel.entry(ck).or_default().list_state.select_previous();
            vec![]
        }
        KeyCode::Char('G') | KeyCode::End => {
            caches.channel.entry(ck).or_default().list_state.select_last();
            vec![]
        }
        KeyCode::Home => {
            caches.channel.entry(ck).or_default().list_state.select_first();
            vec![]
        }
        KeyCode::PageDown => {
            caches.channel.entry(ck).or_default().list_state.scroll_down_by(10);
            vec![]
        }
        KeyCode::PageUp => {
            caches.channel.entry(ck).or_default().list_state.scroll_up_by(10);
            vec![]
        }
        KeyCode::Char('i') => {
            if state.nav.input_mode_allowed() {
                state.nav.input_mode = true;
                state.nav.focus_ring.set(FocusId::InputBox);
            }
            vec![]
        }
        KeyCode::Char('r') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(ch) = state.channels.channels.get(&ck) {
                if let Some(cache) = caches.channel.get(&ck) {
                    if let Some(idx) = cache.list_state.selected() {
                        if let Some(msg) = ch.messages().get(idx) {
                            let input = state.session.channel_inputs
                                .entry(ck).or_insert_with(InputBox::new);
                            input.set_mode(InputMode::Reply {
                                message_id: msg.message_id.clone(),
                                author: msg.author_display_name.clone(),
                            });
                            state.nav.input_mode = true;
                            state.nav.input_context = InputContext::Reply {
                                message_id: msg.message_id.clone(),
                                author: msg.author_display_name.clone(),
                            };
                            state.nav.focus_ring.set(FocusId::InputBox);
                        }
                    }
                }
            }
            vec![]
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                let failed_body = ch.messages().iter().rev()
                    .find(|m| m.delivery_status == rekindle_types::display::DeliveryStatus::Failed)
                    .map(|m| m.body.clone());
                if let Some(body) = failed_body {
                    let failed_reply_to = ch.messages().iter().rev()
                        .find(|m| m.delivery_status == rekindle_types::display::DeliveryStatus::Failed)
                        .and_then(|m| m.reply_to_sequence);
                    ch.retain_messages(|m| m.delivery_status != rekindle_types::display::DeliveryStatus::Failed);
                    let (_, effect) = state.in_flight.track_send(
                        PendingSend::ChannelMessage {
                            community: community.to_string(), channel: channel.to_string(),
                            body: body.clone(), reply_to: failed_reply_to,
                        },
                        rekindle_types::daemon::DaemonRequest::Chat(
                            rekindle_types::daemon::ChatRequest::ChannelSend {
                                community: community.to_string(), channel: channel.to_string(),
                                body: body.clone(), reply_to: failed_reply_to, client_msg_id: None,
                            },
                        ),
                        state.now,
                    );
                    let _ = ch.push_message(rekindle_types::display::DecryptedMessageDisplay {
                        message_id: format!("pending-{}", state.wall_clock_ms),
                        sequence: 0,
                        author_pseudonym: String::new(),
                        author_display_name: "you".into(),
                        body,
                        timestamp: state.wall_clock_ms,
                        reply_to_sequence: None,
                        mek_generation: 0,
                        is_encrypted: false,
                        needs_mek: None,
                        delivery_status: rekindle_types::display::DeliveryStatus::Sending,
                        thread_id: None,
                    });
                    return vec![effect];
                }
            }
            vec![]
        }
        KeyCode::Char('e') => {
            if let Some(ch) = state.channels.channels.get(&ck) {
                if let Some(cache) = caches.channel.get(&ck) {
                    if let Some(idx) = cache.list_state.selected() {
                        if let Some(msg) = ch.messages().get(idx) {
                            if msg.author_pseudonym.is_empty() {
                                let input = state.session.channel_inputs
                                    .entry(ck).or_insert_with(InputBox::new);
                                input.set_mode(InputMode::Edit {
                                    message_id: msg.message_id.clone(),
                                });
                                input.insert_text(&msg.body);
                                state.nav.input_mode = true;
                                state.nav.input_context = InputContext::Edit {
                                    message_id: msg.message_id.clone(),
                                    original: msg.body.clone(),
                                };
                                state.nav.focus_ring.set(FocusId::InputBox);
                            }
                        }
                    }
                }
            }
            vec![]
        }
        KeyCode::Char('R') => {
            if let Some(ch) = state.channels.channels.get(&ck) {
                if let Some(cache) = caches.channel.get(&ck) {
                    if let Some(idx) = cache.list_state.selected() {
                        if let Some(msg) = ch.messages().get(idx) {
                            state.nav.emoji_picker.open_for_message(msg.message_id.clone());
                            state.nav.overlay = Some(OverlayState::EmojiPicker);
                        }
                    }
                }
            }
            vec![]
        }
        KeyCode::Char('x') | KeyCode::Delete => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                ch.retain_messages(|m| m.delivery_status != rekindle_types::display::DeliveryStatus::Failed);
            }
            vec![]
        }
        KeyCode::Char('p') => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                if ch.has_pins() {
                    ch.clear_pins();
                } else {
                    let (_, effect) = state.in_flight.track_request(
                        RequestKind::Pins { community: community.to_string() },
                        rekindle_types::daemon::DaemonRequest::Chat(
                            rekindle_types::daemon::ChatRequest::PinList { community: community.to_string() },
                        ),
                        state.now,
                    );
                    return vec![effect];
                }
            }
            vec![]
        }
        KeyCode::Char('T') => {
            if let Some(ch) = state.channels.channels.get(&ck) {
                if let Some(cache) = caches.channel.get(&ck) {
                    if let Some(idx) = cache.list_state.selected() {
                        if let Some(msg) = ch.messages().get(idx) {
                            if let Some(ref tid) = msg.thread_id {
                                let (_, effect) = state.in_flight.track_request(
                                    RequestKind::ThreadMessages {
                                        community: community.to_string(),
                                        thread_id: tid.clone(),
                                    },
                                    rekindle_types::daemon::DaemonRequest::Chat(
                                        rekindle_types::daemon::ChatRequest::ThreadHistory {
                                            thread_id: tid.clone(), limit: 50,
                                        },
                                    ),
                                    state.now,
                                );
                                return vec![effect];
                            }
                        }
                    }
                }
            }
            vec![]
        }
        KeyCode::Char('q') => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                ch.thread_messages.clear();
            }
            vec![]
        }
        KeyCode::Char('u') => {
            if let Some(ch) = state.channels.channels.get(&ck) {
                if let Some(pins) = ch.pins() {
                    if let Some(cache) = caches.channel.get(&ck) {
                        if let Some(idx) = cache.list_state.selected() {
                            if let Some(msg) = ch.messages().get(idx) {
                                if pins.iter().any(|p| p.message_id == msg.message_id) {
                                    state.nav.confirm.show(
                                        "Unpin this message?",
                                        "The pin will be removed.",
                                        PendingConfirmAction::UnpinMessage {
                                            community: community.to_string(),
                                            channel: channel.to_string(),
                                            message_id: msg.message_id.clone(),
                                        },
                                    );
                                    state.nav.overlay = Some(OverlayState::Confirm);
                                }
                            }
                        }
                    }
                }
            }
            vec![]
        }
        KeyCode::Char('V') => {
            state.voice.active_session = Some(crate::v2::tui::state::voice::ActiveVoiceSession {
                community: community.to_string(),
                channel: channel.to_string(),
                muted: false,
                deafened: false,
                participants: Vec::new(),
            });
            let (_, effect) = state.in_flight.track_request(
                RequestKind::Send,
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::VoiceJoin {
                        community: community.to_string(), channel: channel.to_string(),
                        muted: false, deafened: false,
                    },
                ),
                state.now,
            );
            vec![
                effect,
                Effect::Navigate(ViewKind::VoiceSession {
                    community: community.to_string(),
                    channel: channel.to_string(),
                }),
            ]
        }
        KeyCode::Tab => { state.nav.focus_ring.next(); vec![] }
        _ => vec![],
    }
}

fn handle_input_key(key: &KeyEvent, state: &mut TuiState, community: &str, channel: &str) -> Vec<Effect> {
    let ck = ch_key(community, channel);

    if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.nav.search = Some(crate::v2::tui::state::search::SearchState::new(
            crate::v2::tui::state::search::SearchMode::QuickSwitch,
        ));
        return vec![];
    }

    let input = state.session.channel_inputs
        .entry(ck.clone())
        .or_insert_with(InputBox::new);

    let should_typing = matches!(key.code, KeyCode::Char(_)) && input.should_emit_typing(state.now);
    let mode_before = input.mode().clone();

    match input.handle_key(*key) {
        InputBoxResult::Submit(text) => {
            let mut expanded = text;
            expand_shortcodes(&mut expanded);

            if expanded == "/patch" || expanded.starts_with("/patch ") {
                let args = expanded.strip_prefix("/patch").unwrap_or("").trim();
                let file_paths: Vec<&str> = if args.is_empty() { vec![] } else { args.split_whitespace().collect() };
                let cwd = std::env::current_dir().unwrap_or_default();
                match crate::v2::patch::generate::generate_patch(&cwd, &file_paths, false) {
                    Ok(patch) => {
                        if patch.diff.trim().is_empty() {
                            state.ephemeral.toasts.push("No changes to create a patch from".into(), ToastLevel::Warning, state.now);
                            if let Some(input) = state.session.channel_inputs.get_mut(&ck) { input.clear(); }
                            return vec![];
                        }
                        let desc = if args.is_empty() { String::new() } else { format!("Patch for: {args}\n\n") };
                        expanded = format!("{desc}```patch\n{}\n```", patch.diff);
                    }
                    Err(e) => {
                        state.ephemeral.toasts.push(format!("Patch generation failed: {e}"), ToastLevel::Error, state.now);
                        if let Some(input) = state.session.channel_inputs.get_mut(&ck) { input.clear(); }
                        return vec![];
                    }
                }
            }

            match mode_before {
                InputMode::Edit { message_id } => {
                    if let Some(ch) = state.channels.channels.get_mut(&ck) {
                        ch.edit_message(&message_id, format!("{expanded} (edited)"));
                    }
                    let (_, effect) = state.in_flight.track_request(
                        RequestKind::Send,
                        rekindle_types::daemon::DaemonRequest::Chat(
                            rekindle_types::daemon::ChatRequest::MessageEdit {
                                community: community.to_string(), channel: channel.to_string(),
                                message_id, new_body: expanded,
                            },
                        ),
                        state.now,
                    );
                    if let Some(input) = state.session.channel_inputs.get_mut(&ck) { input.clear(); }
                    vec![effect]
                }
                InputMode::Reply { message_id, .. } => {
                    let reply_seq = state.channels.channels.get(&ck)
                        .and_then(|ch| ch.messages().iter().find(|m| m.message_id == message_id))
                        .map(|m| m.sequence);
                    let (_, effect) = state.in_flight.track_send(
                        PendingSend::ChannelMessage {
                            community: community.to_string(), channel: channel.to_string(),
                            body: expanded.clone(), reply_to: reply_seq,
                        },
                        rekindle_types::daemon::DaemonRequest::Chat(
                            rekindle_types::daemon::ChatRequest::ChannelSend {
                                community: community.to_string(), channel: channel.to_string(),
                                body: expanded.clone(), reply_to: reply_seq, client_msg_id: None,
                            },
                        ),
                        state.now,
                    );
                    if let Some(ch) = state.channels.channels.get_mut(&ck) {
                        let _ = ch.push_message(rekindle_types::display::DecryptedMessageDisplay {
                            message_id: format!("pending-{}", state.wall_clock_ms),
                            sequence: 0, author_pseudonym: String::new(),
                            author_display_name: "you".into(), body: expanded,
                            timestamp: state.wall_clock_ms, reply_to_sequence: reply_seq,
                            mek_generation: 0, is_encrypted: false, needs_mek: None,
                            delivery_status: rekindle_types::display::DeliveryStatus::Sending,
                            thread_id: None,
                        });
                    }
                    if let Some(input) = state.session.channel_inputs.get_mut(&ck) { input.clear(); }
                    vec![effect]
                }
                InputMode::Compose => {
                    let (_, effect) = state.in_flight.track_send(
                        PendingSend::ChannelMessage {
                            community: community.to_string(), channel: channel.to_string(),
                            body: expanded.clone(), reply_to: None,
                        },
                        rekindle_types::daemon::DaemonRequest::Chat(
                            rekindle_types::daemon::ChatRequest::ChannelSend {
                                community: community.to_string(), channel: channel.to_string(),
                                body: expanded.clone(), reply_to: None, client_msg_id: None,
                            },
                        ),
                        state.now,
                    );
                    if let Some(ch) = state.channels.channels.get_mut(&ck) {
                        let _ = ch.push_message(rekindle_types::display::DecryptedMessageDisplay {
                            message_id: format!("pending-{}", state.wall_clock_ms),
                            sequence: 0, author_pseudonym: String::new(),
                            author_display_name: "you".into(), body: expanded,
                            timestamp: state.wall_clock_ms, reply_to_sequence: None,
                            mek_generation: 0, is_encrypted: false, needs_mek: None,
                            delivery_status: rekindle_types::display::DeliveryStatus::Sending,
                            thread_id: None,
                        });
                    }
                    if let Some(input) = state.session.channel_inputs.get_mut(&ck) { input.clear(); }
                    vec![effect]
                }
            }
        }
        InputBoxResult::ExitInputMode => {
            state.nav.input_mode = false;
            if let Some(input) = state.session.channel_inputs.get_mut(&ck) {
                input.set_mode(InputMode::Compose);
            }
            state.nav.focus_ring.set(FocusId::MessageList);
            vec![]
        }
        InputBoxResult::TypingActivity => {
            if should_typing {
                let (_, effect) = state.in_flight.track_request(
                    RequestKind::Typing,
                    rekindle_types::daemon::DaemonRequest::Chat(
                        rekindle_types::daemon::ChatRequest::ChannelTyping {
                            community: community.to_string(),
                            channel: channel.to_string(),
                        },
                    ),
                    state.now,
                );
                vec![effect]
            } else {
                vec![]
            }
        }
        InputBoxResult::OverLimit(msg) => {
            state.ephemeral.toasts.push(msg, ToastLevel::Warning, state.now);
            vec![]
        }
        InputBoxResult::None => vec![],
    }
}

fn handle_peer_key(key: &KeyEvent, state: &mut TuiState, community: &str, channel: &str) -> Vec<Effect> {
    let ck = ch_key(community, channel);
    match key.code {
        KeyCode::Char('h') => {
            // Focus left: peer list → message list
            state.nav.focus_ring.set(FocusId::MessageList);
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                let max = state.communities.members.get(community)
                    .map_or(0, |m| m.len().saturating_sub(1));
                let current = ch.peer_list_selected.unwrap_or(0);
                ch.peer_list_selected = Some((current + 1).min(max));
            }
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                let current = ch.peer_list_selected.unwrap_or(0);
                ch.peer_list_selected = Some(current.saturating_sub(1));
            }
            vec![]
        }
        KeyCode::Enter => {
            let selected = state.channels.channels.get(&ck)
                .and_then(|ch| ch.peer_list_selected);
            if let Some(idx) = selected {
                if let Some(members) = state.communities.members.get(community) {
                    if let Some(member) = members.get(idx) {
                        let peer_key = member.member.pseudonym_key.clone();
                        if let Some(ch) = state.channels.channels.get_mut(&ck) {
                            if ch.active_split_dm.as_deref() == Some(&peer_key) {
                                ch.active_split_dm = None;
                            } else {
                                ch.active_split_dm = Some(peer_key.clone());
                            }
                        }
                    }
                }
            }
            vec![]
        }
        KeyCode::Tab => { state.nav.focus_ring.next(); vec![] }
        _ => vec![],
    }
}

fn handle_split_dm_messages_key(
    key: &KeyEvent,
    state: &mut TuiState,
    community: &str,
    channel: &str,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    let ck = ch_key(community, channel);
    let peer_key = state.channels.channels.get(&ck)
        .and_then(|ch| ch.active_split_dm.clone());
    if let Some(ref pk) = peer_key {
        let cache = caches.split_dm.entry(pk.clone()).or_default();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => { cache.list_state.select_next(); }
            KeyCode::Char('k') | KeyCode::Up => { cache.list_state.select_previous(); }
            KeyCode::Char('G') | KeyCode::End => { cache.list_state.select_last(); }
            KeyCode::Home => { cache.list_state.select_first(); }
            _ => {}
        }
    }
    match key.code {
        KeyCode::Tab => { state.nav.focus_ring.next(); vec![] }
        _ => vec![],
    }
}

fn handle_split_dm_input(
    key: &KeyEvent,
    state: &mut TuiState,
    community: &str,
    channel: &str,
) -> Vec<Effect> {
    let ck = ch_key(community, channel);
    let peer_key = match state.channels.channels.get(&ck).and_then(|ch| ch.active_split_dm.clone()) {
        Some(pk) => pk,
        None => return vec![],
    };

    let input = state.session.split_dm_inputs
        .entry(peer_key.clone())
        .or_insert_with(InputBox::new);

    match input.handle_key(*key) {
        InputBoxResult::Submit(text) => {
            let (_, effect) = state.in_flight.track_send(
                PendingSend::Dm { peer_key: peer_key.clone(), body: text.clone() },
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::DmSend { peer_key: peer_key.clone(), body: text.clone() },
                ),
                state.now,
            );
            if let Some(thread) = state.dm.threads.get_mut(&peer_key) {
                thread.push_message(DmMessage {
                    display: rekindle_types::display::DmMessageDisplay {
                        sender_key: String::new(),
                        sender_name: "you".into(),
                        body: text,
                        timestamp: state.wall_clock_ms,
                        is_self: true,
                        sequence: 0,
                    },
                    delivery_status: rekindle_types::display::DeliveryStatus::Sending,
                });
            }
            vec![effect]
        }
        InputBoxResult::ExitInputMode => {
            state.nav.input_mode = false;
            state.nav.focus_ring.set(FocusId::MessageList);
            vec![]
        }
        _ => vec![],
    }
}

fn handle_thread_panel_key(
    key: &KeyEvent,
    state: &mut TuiState,
    community: &str,
    channel: &str,
) -> Vec<Effect> {
    let ck = ch_key(community, channel);
    match key.code {
        KeyCode::Char('i') => {
            if let Some(ch) = state.channels.channels.get(&ck) {
                if let Some(thread_id) = ch.thread_messages.keys().next().cloned() {
                    state.nav.input_mode = true;
                    state.nav.input_context = InputContext::ThreadCompose {
                        thread_id,
                    };
                    state.nav.focus_ring.set(FocusId::InputBox);
                }
            }
            vec![]
        }
        KeyCode::Char('q') => {
            if let Some(ch) = state.channels.channels.get_mut(&ck) {
                ch.thread_messages.clear();
            }
            state.nav.focus_ring.set(FocusId::MessageList);
            vec![]
        }
        KeyCode::Tab => {
            state.nav.focus_ring.next();
            vec![]
        }
        _ => vec![],
    }
}

fn expand_shortcodes(text: &mut String) {
    let mut pos = 0;
    while pos < text.len() {
        if let Some(start) = text[pos..].find(':') {
            let abs_start = pos + start;
            if let Some(end) = text[abs_start + 1..].find(':') {
                let abs_end = abs_start + 1 + end + 1;
                let shortcode = &text[abs_start..abs_end];
                if let Some(emoji) = EmojiPickerState::resolve_shortcode(shortcode) {
                    text.replace_range(abs_start..abs_end, emoji);
                    pos = abs_start + emoji.len();
                    continue;
                }
            }
            pos = abs_start + 1;
        } else {
            break;
        }
    }
}
