//! Terminal event processing — keyboard routing, mouse hit testing, paste.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};

use crate::v2::tui::components::input_box::InputBoxResult;
use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::keybinds::{KeymapAction, KeymapContext, KeymapStore};
use crate::v2::tui::state::file_search::{FileSearchResult, FileSearchState};
use crate::v2::tui::state::navigation::{InputContext, OverlayState, ViewKind};
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::search::{SearchActionTag, SearchItem, SearchMode, SearchState};
use crate::v2::tui::state::TuiState;
use crate::v2::views;

pub fn process_input(
    event: &TerminalEvent,
    state: &mut TuiState,
    keymap: &KeymapStore,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match event {
        TerminalEvent::Key(key) => route_key(key, state, keymap, caches),
        TerminalEvent::Mouse(mouse) => route_mouse(mouse, state, caches),
        TerminalEvent::Resize(w, h) => {
            tracing::debug!(width = w, height = h, "terminal resized");
            state.render_needed = true;
            vec![]
        }
        TerminalEvent::Paste(text) => {
            if state.nav.input_mode {
                insert_paste(state, text);
            }
            vec![]
        }
        TerminalEvent::FocusGained => {
            state.nav.terminal_focused = true;
            vec![]
        }
        TerminalEvent::FocusLost => {
            state.nav.terminal_focused = false;
            vec![]
        }
    }
}

fn route_key(
    key: &KeyEvent,
    state: &mut TuiState,
    keymap: &KeymapStore,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return vec![Effect::Quit];
    }

    if state.nav.overlay.is_some() {
        return handle_overlay_key(key, state);
    }

    if let Some(ref mut fs) = state.nav.file_search {
        if fs.visible {
            return handle_file_search_key(key, state);
        }
    }

    if let Some(ref mut search_state) = state.nav.search {
        if search_state.visible {
            return handle_search_key(key, state, caches);
        }
    }

    if state.nav.input_mode {
        return handle_input_mode_key(key, state);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c @ '1'..='9') = key.code {
            let index = (c as usize) - ('1' as usize);
            return handle_tab_switch(index, state);
        }
    }

    let context = if state.nav.overlay.is_some() {
        KeymapContext::Overlay
    } else if state.nav.input_mode {
        KeymapContext::Input
    } else {
        KeymapContext::Default
    };

    if let Some(action) = keymap.classify(*key, context) {
        return handle_keymap_action(action, state, caches);
    }

    let view = state.nav.current_view().clone();
    views::dispatch_view_input(&view, &TerminalEvent::Key(*key), state, caches)
}

fn route_mouse(
    mouse: &crossterm::event::MouseEvent,
    state: &mut TuiState,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match mouse.kind {
        MouseEventKind::ScrollDown => {
            let view = state.nav.current_view().clone();
            views::dispatch_view_input(
                &view,
                &TerminalEvent::Mouse(*mouse),
                state,
                caches,
            )
        }
        MouseEventKind::ScrollUp => {
            let view = state.nav.current_view().clone();
            views::dispatch_view_input(
                &view,
                &TerminalEvent::Mouse(*mouse),
                state,
                caches,
            )
        }
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            if mouse.row == state.nav.tab_bar_row {
                if let Some(&(_, _, idx)) = caches.tab_bar_click_regions.iter()
                    .find(|&&(sx, ex, _)| mouse.column >= sx && mouse.column < ex)
                {
                    state.nav.tab_bar.select(idx);
                    return handle_tab_transition(state);
                }
            }

            let tag = state.nav.current_view().tag();
            if let Some(targets) = caches.click_targets.get(&tag) {
                for (&focus_id, rect) in targets {
                    if mouse.column >= rect.x
                        && mouse.column < rect.x + rect.width
                        && mouse.row >= rect.y
                        && mouse.row < rect.y + rect.height
                    {
                        state.nav.focus_ring.set(focus_id);
                        if focus_id == FocusId::InputBox || focus_id == FocusId::SplitDmInput {
                            if state.nav.input_mode_allowed() {
                                state.nav.input_mode = true;
                            }
                            if focus_id == FocusId::SplitDmInput {
                                if let ViewKind::ChannelWatch { community, channel } = state.nav.current_view() {
                                    let key = crate::v2::tui::state::channels::ChannelKey {
                                        community: community.clone(), channel: channel.clone(),
                                    };
                                    if let Some(pk) = state.channels.channels.get(&key)
                                        .and_then(|ch| ch.active_split_dm.clone())
                                    {
                                        state.nav.input_context = InputContext::SplitDmCompose { peer_key: pk };
                                    }
                                }
                            }
                        }
                        return vec![];
                    }
                }
            }
            vec![]
        }
        _ => vec![],
    }
}

fn handle_overlay_key(key: &KeyEvent, state: &mut TuiState) -> Vec<Effect> {
    let overlay = state.nav.overlay.clone();
    match overlay {
        Some(OverlayState::Help) => {
            state.nav.overlay = None;
            vec![]
        }
        Some(OverlayState::Confirm) => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some(action) = state.nav.confirm.take_confirmed() {
                        state.nav.overlay = None;
                        confirm_action_to_effects(action, state)
                    } else {
                        state.nav.overlay = None;
                        vec![]
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    state.nav.confirm.hide();
                    state.nav.overlay = None;
                    vec![]
                }
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    state.nav.confirm.toggle_focus();
                    vec![]
                }
                _ => vec![],
            }
        }
        Some(OverlayState::EmojiPicker) => {
            match key.code {
                KeyCode::Esc => {
                    state.nav.emoji_picker.close();
                    state.nav.overlay = None;
                    vec![]
                }
                KeyCode::Enter => {
                    if let Some(emoji) = state.nav.emoji_picker.selected_emoji() {
                        let emoji = emoji.to_string();
                        state.nav.emoji_picker.close();
                        state.nav.overlay = None;
                        emoji_selection_to_effects(&emoji, state)
                    } else {
                        vec![]
                    }
                }
                KeyCode::Left => { state.nav.emoji_picker.navigate(-1, 0); vec![] }
                KeyCode::Right => { state.nav.emoji_picker.navigate(1, 0); vec![] }
                KeyCode::Up => { state.nav.emoji_picker.navigate(0, -1); vec![] }
                KeyCode::Down => { state.nav.emoji_picker.navigate(0, 1); vec![] }
                KeyCode::Backspace => { state.nav.emoji_picker.backspace(); vec![] }
                KeyCode::Char(c) => { state.nav.emoji_picker.type_char(c); vec![] }
                _ => vec![],
            }
        }
        None => vec![],
    }
}

fn handle_search_key(
    key: &KeyEvent,
    state: &mut TuiState,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match key.code {
        KeyCode::Esc => {
            if let Some(ref mut s) = state.nav.search {
                s.close();
            }
            state.nav.search = None;
            state.nav.input_context = InputContext::Compose;
            state.theme_preview = None;
            vec![]
        }
        KeyCode::Enter => {
            let tag = state.nav.search.as_ref()
                .and_then(|s| s.selected_item_index(&caches.search_list))
                .and_then(|idx| state.nav.search.as_ref().and_then(|s| s.items.get(idx)))
                .map(|item| item.action_tag.clone());
            if let Some(tag) = tag {
                state.nav.search = None;
                state.nav.input_context = InputContext::Compose;
                state.theme_preview = None;
                search_action_to_effects(tag, state, caches)
            } else {
                vec![]
            }
        }
        KeyCode::Down | KeyCode::Tab => {
            if let Some(ref search) = state.nav.search {
                search.move_selection(1, &mut caches.search_list);
            }
            let preview = state.nav.search.as_ref()
                .and_then(|s| extract_theme_preview(s, &caches.search_list));
            state.theme_preview = preview;
            vec![]
        }
        KeyCode::Up | KeyCode::BackTab => {
            if let Some(ref search) = state.nav.search {
                search.move_selection(-1, &mut caches.search_list);
            }
            let preview = state.nav.search.as_ref()
                .and_then(|s| extract_theme_preview(s, &caches.search_list));
            state.theme_preview = preview;
            vec![]
        }
        KeyCode::Char(c) => {
            if let Some(ref mut search) = state.nav.search {
                search.push_char(c);
                caches.search_list.select(if search.filtered_indices.is_empty() { None } else { Some(0) });
            }
            vec![]
        }
        KeyCode::Backspace => {
            if let Some(ref mut search) = state.nav.search {
                search.pop_char();
                caches.search_list.select(if search.filtered_indices.is_empty() { None } else { Some(0) });
            }
            vec![]
        }
        _ => vec![],
    }
}

fn handle_file_search_key(key: &KeyEvent, state: &mut TuiState) -> Vec<Effect> {
    match key.code {
        KeyCode::Esc => {
            state.nav.file_search = None;
            vec![]
        }
        KeyCode::Enter => {
            if let Some(ref fs) = state.nav.file_search {
                if let Some(idx) = fs.selected_index {
                    if let Some(result) = fs.results.get(idx) {
                        let path = result.file_path.clone();
                        let line = Some(result.line_number);
                        state.nav.file_search = None;
                        return vec![Effect::Navigate(ViewKind::FilePreview { path, line })];
                    }
                }
            }
            vec![]
        }
        KeyCode::Down => {
            if let Some(ref mut fs) = state.nav.file_search {
                if let Some(ref mut idx) = fs.selected_index {
                    *idx = (*idx + 1).min(fs.results.len().saturating_sub(1));
                }
            }
            vec![]
        }
        KeyCode::Up => {
            if let Some(ref mut fs) = state.nav.file_search {
                if let Some(ref mut idx) = fs.selected_index {
                    *idx = idx.saturating_sub(1);
                }
            }
            vec![]
        }
        KeyCode::Char(c) => {
            if let Some(ref mut fs) = state.nav.file_search {
                fs.query.push(c);
            }
            let (fs, engine) = (state.nav.file_search.as_mut(), state.search_engine.as_mut());
            if let Some(fs) = fs {
                populate_file_search_results(fs, engine);
            }
            vec![]
        }
        KeyCode::Backspace => {
            if let Some(ref mut fs) = state.nav.file_search {
                fs.query.pop();
            }
            let (fs, engine) = (state.nav.file_search.as_mut(), state.search_engine.as_mut());
            if let Some(fs) = fs {
                populate_file_search_results(fs, engine);
            }
            vec![]
        }
        _ => vec![],
    }
}

fn populate_file_search_results(
    fs: &mut FileSearchState,
    search_engine: Option<&mut crate::v2::search::RekindleSearch>,
) {
    if let Some(engine) = search_engine {
        if fs.query.starts_with('/') {
            let grep_query = &fs.query[1..];
            if !grep_query.is_empty() {
                let results = engine.grep(grep_query, fff_search::GrepMode::Regex, 0, 50);
                fs.results = results.iter().map(|(path, line_num, content)| {
                    FileSearchResult {
                        file_path: path.clone(),
                        line_number: *line_num as usize,
                        line_content: content.clone(),
                    }
                }).collect();
            } else {
                fs.results.clear();
            }
        } else {
            let results = engine.search_files(&fs.query, None, 50);
            fs.results = results.iter().map(|(path, _)| {
                FileSearchResult {
                    file_path: path.clone(),
                    line_number: 0,
                    line_content: String::new(),
                }
            }).collect();
        }
        fs.total_matches = fs.results.len();
        fs.selected_index = if fs.results.is_empty() { None } else { Some(0) };
    }
}

fn handle_input_mode_key(key: &KeyEvent, state: &mut TuiState) -> Vec<Effect> {
    if key.code == KeyCode::Esc {
        state.nav.input_mode = false;
        state.nav.input_context = InputContext::Compose;
        return vec![];
    }

    let view = state.nav.current_view().clone();
    let input = get_active_input(state, &view);
    if let Some(input) = input {
        match input.handle_key(*key) {
            InputBoxResult::Submit(text) => {
                submit_from_view(&view, text, state)
            }
            InputBoxResult::ExitInputMode => {
                state.nav.input_mode = false;
                state.nav.input_context = InputContext::Compose;
                vec![]
            }
            InputBoxResult::TypingActivity => {
                typing_effect_for_view(&view, state)
            }
            InputBoxResult::OverLimit(msg) => {
                state.ephemeral.toasts.push(msg, crate::v2::tui::state::ephemeral::ToastLevel::Warning, state.now);
                vec![]
            }
            InputBoxResult::None => vec![],
        }
    } else {
        vec![]
    }
}

fn handle_tab_switch(index: usize, state: &mut TuiState) -> Vec<Effect> {
    state.nav.tab_bar.select(index);
    handle_tab_transition(state)
}

fn handle_tab_transition(state: &mut TuiState) -> Vec<Effect> {
    let Some(tab_id) = state.nav.tab_bar.selected_id().map(str::to_string) else {
        tracing::warn!("tui: handle_tab_transition — no tab selected");
        return vec![];
    };
    tracing::info!(tab_id, "tui: tab transition");
    state.nav.tab_bar.clear_unread(&tab_id);
    let view = match tab_id.as_str() {
        "dashboard" => ViewKind::Dashboard,
        "communities" => {
            if let Some(first) = state.communities.list.first() {
                ViewKind::CommunityInfo { community: first.governance_key.clone() }
            } else {
                return vec![];
            }
        }
        "dms" => ViewKind::DmInbox,
        "friends" => ViewKind::FriendList,
        _ => return vec![],
    };
    vec![Effect::Navigate(view)]
}

fn handle_keymap_action(
    action: KeymapAction,
    state: &mut TuiState,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    tracing::debug!(action = ?action, "tui: keymap action");
    match action {
        KeymapAction::Quit => {
            if matches!(state.nav.current_view(), ViewKind::Dashboard) {
                vec![Effect::SaveSession, Effect::Quit]
            } else {
                state.nav.pop_view();
                vec![]
            }
        }
        KeymapAction::FocusNext => { state.nav.focus_ring.next(); vec![] }
        KeymapAction::FocusPrev => { state.nav.focus_ring.prev(); vec![] }
        KeymapAction::EnterInputMode => {
            if state.nav.input_mode_allowed() {
                state.nav.input_mode = true;
                state.nav.input_context = InputContext::Compose;
            }
            vec![]
        }
        KeymapAction::ToggleSidebar => { state.nav.sidebar_visible = !state.nav.sidebar_visible; vec![] }
        KeymapAction::ToggleHelp => {
            state.nav.overlay = if state.nav.overlay.is_some() { None } else { Some(OverlayState::Help) };
            vec![]
        }
        KeymapAction::NextTab => {
            state.nav.tab_bar.next();
            handle_tab_transition(state)
        }
        KeymapAction::PrevTab => {
            state.nav.tab_bar.prev();
            handle_tab_transition(state)
        }
        KeymapAction::Back => { state.nav.pop_view(); vec![] }
        KeymapAction::Cancel => {
            if state.nav.overlay.is_some() {
                state.nav.overlay = None;
            } else if state.nav.search.is_some() {
                state.nav.search = None;
            } else if state.nav.file_search.is_some() {
                state.nav.file_search = None;
            } else if state.nav.input_mode {
                state.nav.input_mode = false;
            } else if !state.ephemeral.rails.dismiss_first_dismissible() {
                state.ephemeral.toasts.dismiss_oldest();
            }
            vec![]
        }
        KeymapAction::Refresh => {
            if let Some(ref engine) = state.search_engine {
                let _ = engine.rescan();
            }
            let view = state.nav.current_view().clone();
            vec![Effect::Navigate(view)]
        }
        KeymapAction::ShowDashboard => vec![Effect::Navigate(ViewKind::Dashboard)],
        KeymapAction::ShowDmInbox => vec![Effect::Navigate(ViewKind::DmInbox)],
        KeymapAction::ShowFriendList => vec![Effect::Navigate(ViewKind::FriendList)],
        KeymapAction::ShowDoctor => vec![Effect::Navigate(ViewKind::Doctor)],
        KeymapAction::ShowIdentitySettings => vec![Effect::Navigate(ViewKind::IdentitySettings)],
        KeymapAction::OpenSearch(mode) => {
            let items = build_search_items(state, mode);
            let mut search_state = SearchState::new(mode);
            search_state.open(mode, items);
            state.nav.search = Some(search_state);
            state.nav.input_context = InputContext::Search;
            vec![]
        }
        KeymapAction::OpenQuickSwitcher => {
            let items = build_search_items(state, SearchMode::QuickSwitch);
            let mut search_state = SearchState::new(SearchMode::QuickSwitch);
            search_state.open(SearchMode::QuickSwitch, items);
            state.nav.search = Some(search_state);
            state.nav.input_context = InputContext::Search;
            vec![]
        }
        KeymapAction::OpenFileContentSearch => {
            state.nav.file_search = Some(FileSearchState::new());
            vec![]
        }
        KeymapAction::ScrollDown(count) => {
            match state.nav.current_view() {
                ViewKind::ChannelWatch { community, channel } => {
                    let key = crate::v2::tui::state::channels::ChannelKey {
                        community: community.clone(), channel: channel.clone(),
                    };
                    caches.channel.entry(key).or_default().list_state.scroll_down_by(count);
                }
                ViewKind::DmThread { peer_key } => {
                    caches.dm_thread.entry(peer_key.clone()).or_default().list_state.scroll_down_by(count);
                }
                _ => {}
            }
            vec![]
        }
        KeymapAction::ScrollUp(count) => {
            match state.nav.current_view() {
                ViewKind::ChannelWatch { community, channel } => {
                    let key = crate::v2::tui::state::channels::ChannelKey {
                        community: community.clone(), channel: channel.clone(),
                    };
                    caches.channel.entry(key).or_default().list_state.scroll_up_by(count);
                }
                ViewKind::DmThread { peer_key } => {
                    caches.dm_thread.entry(peer_key.clone()).or_default().list_state.scroll_up_by(count);
                }
                _ => {}
            }
            vec![]
        }
        KeymapAction::ScrollToBottom => {
            match state.nav.current_view() {
                ViewKind::ChannelWatch { community, channel } => {
                    let key = crate::v2::tui::state::channels::ChannelKey {
                        community: community.clone(), channel: channel.clone(),
                    };
                    caches.channel.entry(key).or_default().list_state.select_last();
                }
                ViewKind::DmThread { peer_key } => {
                    caches.dm_thread.entry(peer_key.clone()).or_default().list_state.select_last();
                }
                _ => {}
            }
            vec![]
        }
        KeymapAction::ScrollToTop => {
            match state.nav.current_view() {
                ViewKind::ChannelWatch { community, channel } => {
                    let key = crate::v2::tui::state::channels::ChannelKey {
                        community: community.clone(), channel: channel.clone(),
                    };
                    caches.channel.entry(key).or_default().list_state.select_first();
                }
                ViewKind::DmThread { peer_key } => {
                    caches.dm_thread.entry(peer_key.clone()).or_default().list_state.select_first();
                }
                _ => {}
            }
            vec![]
        }
        KeymapAction::ScrollPageDown => {
            match state.nav.current_view() {
                ViewKind::ChannelWatch { community, channel } => {
                    let key = crate::v2::tui::state::channels::ChannelKey {
                        community: community.clone(), channel: channel.clone(),
                    };
                    caches.channel.entry(key).or_default().list_state.scroll_down_by(20);
                }
                _ => {}
            }
            vec![]
        }
        KeymapAction::ScrollPageUp => {
            match state.nav.current_view() {
                ViewKind::ChannelWatch { community, channel } => {
                    let key = crate::v2::tui::state::channels::ChannelKey {
                        community: community.clone(), channel: channel.clone(),
                    };
                    caches.channel.entry(key).or_default().list_state.scroll_up_by(20);
                }
                _ => {}
            }
            vec![]
        }
        KeymapAction::ToggleTimezone => {
            state.timezone.toggle();
            state.render_needed = true;
            vec![]
        }
        KeymapAction::Select | KeymapAction::InputSubmit
        | KeymapAction::ReplyToSelected | KeymapAction::EditSelected
        | KeymapAction::CloseSplitDm | KeymapAction::ExitInputMode => {
            vec![]
        }
    }
}

fn get_active_input<'a>(
    state: &'a mut TuiState,
    view: &ViewKind,
) -> Option<&'a mut crate::v2::tui::components::input_box::InputBox> {
    match view {
        ViewKind::DmInbox => {
            let peer = state.session.dm_selected_peer.clone()?;
            Some(state.session.dm_inputs.entry(peer).or_insert_with(
                crate::v2::tui::components::input_box::InputBox::new,
            ))
        }
        ViewKind::DmThread { peer_key } => {
            Some(state.session.dm_inputs.entry(peer_key.clone()).or_insert_with(
                crate::v2::tui::components::input_box::InputBox::new,
            ))
        }
        ViewKind::ChannelWatch { community, channel } => {
            if let InputContext::ThreadCompose { ref thread_id } = state.nav.input_context {
                return Some(state.session.thread_inputs.entry(thread_id.clone()).or_insert_with(
                    crate::v2::tui::components::input_box::InputBox::new,
                ));
            }
            let key = crate::v2::tui::state::channels::ChannelKey {
                community: community.clone(),
                channel: channel.clone(),
            };
            Some(state.session.channel_inputs.entry(key).or_insert_with(
                crate::v2::tui::components::input_box::InputBox::new,
            ))
        }
        _ => None,
    }
}

fn submit_from_view(view: &ViewKind, text: String, state: &mut TuiState) -> Vec<Effect> {
    tracing::info!(view = ?view, body_len = text.len(), "tui: submit_from_view");
    match view {
        ViewKind::DmInbox => {
            if let Some(peer_key) = state.session.dm_selected_peer.clone() {
                let (_, effect) = state.in_flight.track_send(
                    crate::v2::tui::state::in_flight::PendingSend::Dm {
                        peer_key: peer_key.clone(), body: text.clone(),
                    },
                    rekindle_types::daemon::DaemonRequest::Chat(
                        rekindle_types::daemon::ChatRequest::DmSend { peer_key: peer_key.clone(), body: text.clone() },
                    ),
                    state.now,
                );
                if let Some(thread) = state.dm.threads.get_mut(&peer_key) {
                    thread.push_message(crate::v2::tui::state::dm::DmMessage {
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
            } else {
                vec![]
            }
        }
        ViewKind::DmThread { peer_key } => {
            let (_, effect) = state.in_flight.track_send(
                crate::v2::tui::state::in_flight::PendingSend::Dm {
                    peer_key: peer_key.clone(), body: text.clone(),
                },
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::DmSend { peer_key: peer_key.clone(), body: text.clone() },
                ),
                state.now,
            );
            if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                thread.push_message(crate::v2::tui::state::dm::DmMessage {
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
        ViewKind::ChannelWatch { community, channel } => {
            if let InputContext::ThreadCompose { ref thread_id } = state.nav.input_context {
                let thread_id = thread_id.clone();
                state.nav.input_context = InputContext::Compose;
                let (_, effect) = state.in_flight.track_request(
                    crate::v2::tui::state::in_flight::RequestKind::Send,
                    rekindle_types::daemon::DaemonRequest::Chat(
                        rekindle_types::daemon::ChatRequest::ThreadSend {
                            community: community.clone(),
                            channel: channel.clone(),
                            thread_id,
                            body: text,
                        },
                    ),
                    state.now,
                );
                return vec![effect];
            }
            let (_, effect) = state.in_flight.track_send(
                crate::v2::tui::state::in_flight::PendingSend::ChannelMessage {
                    community: community.clone(), channel: channel.clone(),
                    body: text.clone(), reply_to: None,
                },
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::ChannelSend {
                        community: community.clone(), channel: channel.clone(),
                        body: text.clone(), reply_to: None, client_msg_id: None,
                    },
                ),
                state.now,
            );
            let key = crate::v2::tui::state::channels::ChannelKey {
                community: community.clone(), channel: channel.clone(),
            };
            if let Some(ch) = state.channels.channels.get_mut(&key) {
                let _ = ch.push_message(rekindle_types::display::DecryptedMessageDisplay {
                    message_id: format!("pending-{}", state.wall_clock_ms),
                    sequence: 0,
                    author_pseudonym: String::new(),
                    author_display_name: "you".into(),
                    body: text,
                    timestamp: state.wall_clock_ms,
                    reply_to_sequence: None,
                    mek_generation: 0,
                    is_encrypted: false,
                    needs_mek: None,
                    delivery_status: rekindle_types::display::DeliveryStatus::Sending,
                    thread_id: None,
                });
            }
            vec![effect]
        }
        _ => vec![],
    }
}

fn typing_effect_for_view(view: &ViewKind, state: &mut TuiState) -> Vec<Effect> {
    use crate::v2::tui::state::in_flight::RequestKind;
    match view {
        ViewKind::DmInbox | ViewKind::DmThread { .. } => {
            let peer_key = match view {
                ViewKind::DmInbox => state.session.dm_selected_peer.clone(),
                ViewKind::DmThread { peer_key } => Some(peer_key.clone()),
                _ => None,
            };
            if let Some(pk) = peer_key {
                let (_, effect) = state.in_flight.track_request(
                    RequestKind::Typing,
                    rekindle_types::daemon::DaemonRequest::Chat(
                        rekindle_types::daemon::ChatRequest::DmTyping { peer_key: pk, typing: true },
                    ),
                    state.now,
                );
                vec![effect]
            } else {
                vec![]
            }
        }
        ViewKind::ChannelWatch { community, channel } => {
            let (_, effect) = state.in_flight.track_request(
                RequestKind::Typing,
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::ChannelTyping {
                        community: community.clone(),
                        channel: channel.clone(),
                    },
                ),
                state.now,
            );
            vec![effect]
        }
        _ => vec![],
    }
}

fn insert_paste(state: &mut TuiState, text: &str) {
    let view = state.nav.current_view().clone();
    if let Some(input) = get_active_input(state, &view) {
        input.insert_text(text);
    }
}

fn confirm_action_to_effects(
    action: crate::v2::tui::state::confirm::PendingConfirmAction,
    state: &mut TuiState,
) -> Vec<Effect> {
    use crate::v2::tui::state::confirm::PendingConfirmAction;
    use crate::v2::tui::state::in_flight::RequestKind;
    let request = match action {
        PendingConfirmAction::LeaveCommunity { community } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::CommunityLeave { governance_key: community }),
        PendingConfirmAction::RemoveFriend { peer_key } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::FriendRemove { public_key: peer_key }),
        PendingConfirmAction::KickMember { community, pseudonym } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::Kick { community, target_pseudonym: pseudonym }),
        PendingConfirmAction::BanMember { community, pseudonym } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::Ban { community, target_pseudonym: pseudonym, reason: None }),
        PendingConfirmAction::TimeoutMember { community, pseudonym, duration_secs } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::Timeout { community, target_pseudonym: pseudonym, duration_seconds: duration_secs, reason: None }),
        PendingConfirmAction::ApproveMember { community, pseudonym } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::CommunityApprove { governance_key: community, member_pseudonym: pseudonym }),
        PendingConfirmAction::RejectMember { community, pseudonym } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::CommunityReject { governance_key: community, member_pseudonym: pseudonym, reason: String::new() }),
        PendingConfirmAction::RevokeInvite { community, invite_code } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::InviteRevoke { community, invite_code }),
        PendingConfirmAction::UnpinMessage { community, channel, message_id } =>
            rekindle_types::daemon::DaemonRequest::Chat(rekindle_types::daemon::ChatRequest::PinRemove { community, channel, message_id }),
        PendingConfirmAction::LeaveVoice => {
            return vec![Effect::Navigate(ViewKind::Dashboard)];
        }
    };
    let (_, effect) = state.in_flight.track_request(RequestKind::Send, request, state.now);
    vec![effect]
}

fn emoji_selection_to_effects(emoji: &str, state: &mut TuiState) -> Vec<Effect> {
    if let ViewKind::ChannelWatch { community, channel } = state.nav.current_view() {
        let community = community.clone();
        let channel = channel.clone();
        let channel_id = {
            let key = crate::v2::tui::state::channels::ChannelKey {
                community: community.clone(),
                channel: channel.clone(),
            };
            state.channels.channels.get(&key)
                .and_then(|ch| ch.channel_id.clone())
                .unwrap_or_else(|| channel.clone())
        };
        let (_, effect) = state.in_flight.track_request(
            crate::v2::tui::state::in_flight::RequestKind::Send,
            rekindle_types::daemon::DaemonRequest::Chat(
                rekindle_types::daemon::ChatRequest::ReactionAdd {
                    community,
                    channel: channel_id,
                    message_id: state.nav.emoji_picker.target_message_id.clone(),
                    emoji: emoji.to_string(),
                },
            ),
            state.now,
        );
        vec![effect]
    } else {
        vec![]
    }
}

fn search_action_to_effects(
    tag: SearchActionTag,
    state: &mut TuiState,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match tag {
        SearchActionTag::Navigate(view) => vec![Effect::Navigate(view)],
        SearchActionTag::ScrollToMessage { message_id } => {
            match state.nav.current_view() {
                ViewKind::ChannelWatch { community, channel } => {
                    let key = crate::v2::tui::state::channels::ChannelKey {
                        community: community.clone(), channel: channel.clone(),
                    };
                    if let Some(ch) = state.channels.channels.get(&key) {
                        if let Some(idx) = ch.messages().iter().position(|m| m.message_id == message_id) {
                            caches.channel.entry(key.clone()).or_default().list_state.select(Some(idx));
                        }
                    }
                }
                _ => {}
            }
            vec![]
        }
        SearchActionTag::FileSelected { path } => {
            if let Some(ref engine) = state.search_engine {
                engine.on_open("", &path);
            }
            vec![Effect::Navigate(ViewKind::FilePreview { path, line: None })]
        }
        SearchActionTag::OpenSearch(mode) => {
            let items = build_search_items(state, mode);
            let has_items = !items.is_empty();
            let mut search_state = SearchState::new(mode);
            search_state.open(mode, items);
            state.nav.search = Some(search_state);
            caches.search_list.select(if has_items { Some(0) } else { None });
            vec![]
        }
        SearchActionTag::SetTheme { name } => {
            vec![Effect::SetTheme { name }]
        }
        SearchActionTag::Toggle(action) => {
            use crate::v2::tui::state::search::ToggleAction;
            match action {
                ToggleAction::Sidebar => {
                    state.nav.sidebar_visible = !state.nav.sidebar_visible;
                }
                ToggleAction::Timezone => {
                    state.timezone.toggle();
                }
                ToggleAction::Help => {
                    state.nav.overlay = if state.nav.overlay.is_some() {
                        None
                    } else {
                        Some(OverlayState::Help)
                    };
                }
            }
            state.render_needed = true;
            vec![]
        }
        SearchActionTag::CopyToClipboard(text) => {
            vec![Effect::SetClipboard(text)]
        }
        SearchActionTag::PinCommunityTheme { community, theme } => {
            state.session.community_themes.insert(community, theme);
            state.render_needed = true;
            vec![Effect::SaveSession]
        }
        SearchActionTag::PinDmTheme { peer_key, theme } => {
            state.session.dm_themes.insert(peer_key, theme);
            state.render_needed = true;
            vec![Effect::SaveSession]
        }
        SearchActionTag::UnpinCommunityTheme { community } => {
            state.session.community_themes.remove(&community);
            state.render_needed = true;
            vec![Effect::SaveSession]
        }
        SearchActionTag::UnpinDmTheme { peer_key } => {
            state.session.dm_themes.remove(&peer_key);
            state.render_needed = true;
            vec![Effect::SaveSession]
        }
    }
}

fn build_search_items(state: &TuiState, mode: SearchMode) -> Vec<SearchItem> {
    let mut items = Vec::new();

    match mode {
        SearchMode::QuickSwitch => {
            for community in &state.communities.list {
                items.push(SearchItem {
                    label: community.name.clone(),
                    detail: "community".into(),
                    action_tag: SearchActionTag::Navigate(ViewKind::CommunityInfo {
                        community: community.governance_key.clone(),
                    }),
                });
                if let Some(detail) = state.communities.details.get(&community.governance_key) {
                    for ch in &detail.channels {
                        items.push(SearchItem {
                            label: format!("#{}", ch.name),
                            detail: community.name.clone(),
                            action_tag: SearchActionTag::Navigate(ViewKind::ChannelWatch {
                                community: community.governance_key.clone(),
                                channel: ch.name.clone(),
                            }),
                        });
                    }
                }
            }
            for (peer_key, thread) in state.dm.threads.iter() {
                items.push(SearchItem {
                    label: thread.peer_name.clone(),
                    detail: "dm".into(),
                    action_tag: SearchActionTag::Navigate(ViewKind::DmThread {
                        peer_key: peer_key.clone(),
                    }),
                });
            }
            for friend in &state.friends.friends {
                if !state.dm.threads.contains_key(&friend.public_key) {
                    items.push(SearchItem {
                        label: friend.display_name.clone(),
                        detail: "friend".into(),
                        action_tag: SearchActionTag::Navigate(ViewKind::DmThread {
                            peer_key: friend.public_key.clone(),
                        }),
                    });
                }
            }
            if let Some(ref engine) = state.search_engine {
                for (path, _score) in engine.search_files("", None, 20) {
                    items.push(SearchItem {
                        label: path.clone(),
                        detail: "file".into(),
                        action_tag: SearchActionTag::FileSelected { path },
                    });
                }
            }
        }
        SearchMode::CommandPalette => {
            // ── Navigation ──────────────────────────────────────
            items.push(SearchItem {
                label: "Dashboard".into(),
                detail: "navigate".into(),
                action_tag: SearchActionTag::Navigate(ViewKind::Dashboard),
            });
            items.push(SearchItem {
                label: "DM Inbox".into(),
                detail: "navigate".into(),
                action_tag: SearchActionTag::Navigate(ViewKind::DmInbox),
            });
            items.push(SearchItem {
                label: "Friend List".into(),
                detail: "navigate".into(),
                action_tag: SearchActionTag::Navigate(ViewKind::FriendList),
            });
            items.push(SearchItem {
                label: "Doctor".into(),
                detail: "navigate".into(),
                action_tag: SearchActionTag::Navigate(ViewKind::Doctor),
            });
            items.push(SearchItem {
                label: "Identity Settings".into(),
                detail: "navigate".into(),
                action_tag: SearchActionTag::Navigate(ViewKind::IdentitySettings),
            });

            // ── Community navigation ────────────────────────────
            for community in &state.communities.list {
                items.push(SearchItem {
                    label: format!("Community: {}", community.name),
                    detail: "navigate".into(),
                    action_tag: SearchActionTag::Navigate(ViewKind::CommunityInfo {
                        community: community.governance_key.clone(),
                    }),
                });
                items.push(SearchItem {
                    label: format!("Moderation: {}", community.name),
                    detail: "navigate".into(),
                    action_tag: SearchActionTag::Navigate(ViewKind::Moderation {
                        community: community.governance_key.clone(),
                    }),
                });
                items.push(SearchItem {
                    label: format!("Invites: {}", community.name),
                    detail: "navigate".into(),
                    action_tag: SearchActionTag::Navigate(ViewKind::Invite {
                        community: community.governance_key.clone(),
                    }),
                });
                items.push(SearchItem {
                    label: format!("Events: {}", community.name),
                    detail: "navigate".into(),
                    action_tag: SearchActionTag::Navigate(ViewKind::Events {
                        community: community.governance_key.clone(),
                    }),
                });
                if let Some(detail) = state.communities.details.get(&community.governance_key) {
                    for ch in &detail.channels {
                        items.push(SearchItem {
                            label: format!("#{} in {}", ch.name, community.name),
                            detail: "channel".into(),
                            action_tag: SearchActionTag::Navigate(ViewKind::ChannelWatch {
                                community: community.governance_key.clone(),
                                channel: ch.name.clone(),
                            }),
                        });
                    }
                }
            }

            // ── DM threads ──────────────────────────────────────
            for (peer_key, thread) in state.dm.threads.iter() {
                items.push(SearchItem {
                    label: format!("DM: {}", thread.peer_name),
                    detail: "message".into(),
                    action_tag: SearchActionTag::Navigate(ViewKind::DmThread {
                        peer_key: peer_key.clone(),
                    }),
                });
            }

            // ── Search modes ────────────────────────────────────
            items.push(SearchItem {
                label: "Search Messages".into(),
                detail: "search".into(),
                action_tag: SearchActionTag::OpenSearch(SearchMode::MessageSearch),
            });
            items.push(SearchItem {
                label: "Quick Switch".into(),
                detail: "search".into(),
                action_tag: SearchActionTag::OpenSearch(SearchMode::QuickSwitch),
            });

            // ── Themes ──────────────────────────────────────────
            for name in crate::v2::tui::theme::ThemeManager::available_themes() {
                items.push(SearchItem {
                    label: format!("Theme: {name}"),
                    detail: "appearance".into(),
                    action_tag: SearchActionTag::SetTheme { name: name.to_string() },
                });
            }

            // ── Theme pins for current context ──────────────────
            match state.nav.current_view() {
                ViewKind::ChannelWatch { community, .. }
                | ViewKind::CommunityInfo { community }
                | ViewKind::Moderation { community } => {
                    let community_name = state.communities.name_for(community);
                    for name in crate::v2::tui::theme::ThemeManager::available_themes() {
                        items.push(SearchItem {
                            label: format!("Pin Theme: {name} \u{2192} {community_name}"),
                            detail: "community theme".into(),
                            action_tag: SearchActionTag::PinCommunityTheme {
                                community: community.clone(),
                                theme: name.to_string(),
                            },
                        });
                    }
                    if state.session.community_themes.contains_key(community) {
                        items.push(SearchItem {
                            label: format!("Unpin Theme: {community_name}"),
                            detail: "community theme".into(),
                            action_tag: SearchActionTag::UnpinCommunityTheme {
                                community: community.clone(),
                            },
                        });
                    }
                }
                ViewKind::DmThread { peer_key } => {
                    let peer_name = state.dm.threads.get(peer_key)
                        .map_or("DM", |t| t.peer_name.as_str());
                    for name in crate::v2::tui::theme::ThemeManager::available_themes() {
                        items.push(SearchItem {
                            label: format!("Pin Theme: {name} \u{2192} {peer_name}"),
                            detail: "dm theme".into(),
                            action_tag: SearchActionTag::PinDmTheme {
                                peer_key: peer_key.clone(),
                                theme: name.to_string(),
                            },
                        });
                    }
                    if state.session.dm_themes.contains_key(peer_key) {
                        items.push(SearchItem {
                            label: format!("Unpin Theme: {peer_name}"),
                            detail: "dm theme".into(),
                            action_tag: SearchActionTag::UnpinDmTheme {
                                peer_key: peer_key.clone(),
                            },
                        });
                    }
                }
                ViewKind::DmInbox => {
                    if let Some(ref pk) = state.session.dm_selected_peer {
                        let peer_name = state.dm.threads.get(pk)
                            .map_or("DM", |t| t.peer_name.as_str());
                        for name in crate::v2::tui::theme::ThemeManager::available_themes() {
                            items.push(SearchItem {
                                label: format!("Pin Theme: {name} \u{2192} {peer_name}"),
                                detail: "dm theme".into(),
                                action_tag: SearchActionTag::PinDmTheme {
                                    peer_key: pk.clone(),
                                    theme: name.to_string(),
                                },
                            });
                        }
                        if state.session.dm_themes.contains_key(pk) {
                            items.push(SearchItem {
                                label: format!("Unpin Theme: {peer_name}"),
                                detail: "dm theme".into(),
                                action_tag: SearchActionTag::UnpinDmTheme {
                                    peer_key: pk.clone(),
                                },
                            });
                        }
                    }
                }
                _ => {}
            }

            // ── Toggles ─────────────────────────────────────────
            items.push(SearchItem {
                label: "Toggle Sidebar".into(),
                detail: "setting".into(),
                action_tag: SearchActionTag::Toggle(crate::v2::tui::state::search::ToggleAction::Sidebar),
            });
            items.push(SearchItem {
                label: "Toggle Timezone (UTC/Local)".into(),
                detail: "setting".into(),
                action_tag: SearchActionTag::Toggle(crate::v2::tui::state::search::ToggleAction::Timezone),
            });
            items.push(SearchItem {
                label: "Toggle Help".into(),
                detail: "setting".into(),
                action_tag: SearchActionTag::Toggle(crate::v2::tui::state::search::ToggleAction::Help),
            });

            // ── Clipboard ───────────────────────────────────────
            if let Some(ref id) = state.ephemeral.identity {
                items.push(SearchItem {
                    label: "Copy Public Key".into(),
                    detail: "clipboard".into(),
                    action_tag: SearchActionTag::CopyToClipboard(id.public_key.clone()),
                });
            }

            // ── File search ─────────────────────────────────────
            if let Some(ref engine) = state.search_engine {
                for (path, _score) in engine.search_mixed("", None, 10) {
                    items.push(SearchItem {
                        label: path.clone(),
                        detail: "file".into(),
                        action_tag: SearchActionTag::FileSelected { path },
                    });
                }
            }
        }
        SearchMode::MessageSearch => {
            if let ViewKind::ChannelWatch { community, channel } = state.nav.current_view() {
                let key = crate::v2::tui::state::channels::ChannelKey {
                    community: community.clone(),
                    channel: channel.clone(),
                };
                if let Some(ch) = state.channels.channels.get(&key) {
                    for msg in ch.messages().iter() {
                        if msg.body.is_empty() || msg.body == "(decrypting...)" {
                            continue;
                        }
                        let preview = if msg.body.len() > 80 {
                            format!("{}...", &msg.body[..77])
                        } else {
                            msg.body.clone()
                        };
                        items.push(SearchItem {
                            label: preview,
                            detail: msg.author_display_name.clone(),
                            action_tag: SearchActionTag::ScrollToMessage {
                                message_id: msg.message_id.clone(),
                            },
                        });
                    }
                }
            }
            if let ViewKind::DmInbox | ViewKind::DmThread { .. } = state.nav.current_view() {
                let peer_key = match state.nav.current_view() {
                    ViewKind::DmThread { peer_key } => Some(peer_key.clone()),
                    ViewKind::DmInbox => state.session.dm_selected_peer.clone(),
                    _ => None,
                };
                if let Some(pk) = peer_key {
                    if let Some(thread) = state.dm.threads.get(&pk) {
                        for msg in thread.messages() {
                            if msg.display.body.is_empty() || msg.display.body == "(decrypting...)" {
                                continue;
                            }
                            let preview = if msg.display.body.len() > 80 {
                                format!("{}...", &msg.display.body[..77])
                            } else {
                                msg.display.body.clone()
                            };
                            items.push(SearchItem {
                                label: preview,
                                detail: msg.display.sender_name.clone(),
                                action_tag: SearchActionTag::ScrollToMessage {
                                    message_id: String::new(),
                                },
                            });
                        }
                    }
                }
            }
            if let Some(ref engine) = state.search_engine {
                for query in engine.recent_queries(5) {
                    items.push(SearchItem {
                        label: format!("\u{1f50d} {query}"),
                        detail: "recent search".into(),
                        action_tag: SearchActionTag::OpenSearch(SearchMode::MessageSearch),
                    });
                }
            }
        }
    }

    items
}

fn extract_theme_preview(
    search: &SearchState,
    list_state: &ratatui::widgets::ListState,
) -> Option<String> {
    if search.mode != SearchMode::CommandPalette {
        return None;
    }
    search.selected_item_index(list_state)
        .and_then(|idx| search.items.get(idx))
        .and_then(|item| match &item.action_tag {
            SearchActionTag::SetTheme { name } => Some(name.clone()),
            SearchActionTag::PinCommunityTheme { theme, .. } => Some(theme.clone()),
            SearchActionTag::PinDmTheme { theme, .. } => Some(theme.clone()),
            _ => None,
        })
}
