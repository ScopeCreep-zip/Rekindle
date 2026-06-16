//! DM inbox input — thread list navigation, compose, send, typing indicators.

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};

use crate::v2::tui::components::input_box::{InputBox, InputBoxResult};
use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::dm::DmMessage;
use crate::v2::tui::state::in_flight::{PendingSend, RequestKind};
use crate::v2::tui::state::navigation::{ViewKind, ViewKindTag};
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match event {
        TerminalEvent::Key(key) => {
            match state.nav.focus_ring.current() {
                FocusId::DmList => handle_list_key(key, state),
                FocusId::MessageList => handle_message_key(key, state),
                FocusId::InputBox => handle_input_key(key, state),
                _ => vec![],
            }
        }
        TerminalEvent::Mouse(mouse) => {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                if let Some(targets) = caches.click_targets.get(&ViewKindTag::DmInbox) {
                    for (&focus_id, rect) in targets {
                        if mouse.column >= rect.x && mouse.column < rect.x + rect.width
                            && mouse.row >= rect.y && mouse.row < rect.y + rect.height
                        {
                            state.nav.focus_ring.set(focus_id);
                            if focus_id == FocusId::InputBox {
                                state.nav.input_mode = true;
                            }
                            return vec![];
                        }
                    }
                }
            }
            vec![]
        }
        _ => vec![],
    }
}

fn handle_list_key(key: &KeyEvent, state: &mut TuiState) -> Vec<Effect> {
    match key.code {
        KeyCode::Char('h') => {
            state.nav.pop_view();
            vec![]
        }
        KeyCode::Char('l') => {
            // Same as Enter — open selected thread
            if let Some(pk) = state.session.dm_selected_peer.clone() {
                vec![Effect::Navigate(ViewKind::DmThread { peer_key: pk })]
            } else {
                vec![]
            }
        }
        KeyCode::Char('G') => {
            let max = state.dm.threads.len().saturating_sub(1);
            set_selected_by_index(state, max);
            load_selected_if_needed(state)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let current = selected_index(state).unwrap_or(0);
            let max = state.dm.threads.len().saturating_sub(1);
            let new = (current + 1).min(max);
            set_selected_by_index(state, new);
            load_selected_if_needed(state)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let current = selected_index(state).unwrap_or(0);
            let new = current.saturating_sub(1);
            set_selected_by_index(state, new);
            load_selected_if_needed(state)
        }
        KeyCode::Enter => {
            if let Some(pk) = state.session.dm_selected_peer.clone() {
                vec![Effect::Navigate(ViewKind::DmThread { peer_key: pk })]
            } else {
                vec![]
            }
        }
        KeyCode::Tab => {
            state.nav.focus_ring.next();
            vec![]
        }
        _ => vec![],
    }
}

fn handle_message_key(key: &KeyEvent, state: &mut TuiState) -> Vec<Effect> {
    match key.code {
        KeyCode::Char('i') => {
            if state.nav.input_mode_allowed() {
                state.nav.input_mode = true;
                state.nav.focus_ring.set(FocusId::InputBox);
            }
            vec![]
        }
        KeyCode::Tab => {
            state.nav.focus_ring.next();
            vec![]
        }
        _ => vec![],
    }
}

fn handle_input_key(key: &KeyEvent, state: &mut TuiState) -> Vec<Effect> {
    let peer_key = match state.session.dm_selected_peer.clone() {
        Some(pk) => pk,
        None => return vec![],
    };

    let input = state.session.dm_inputs
        .entry(peer_key.clone())
        .or_insert_with(InputBox::new);

    let should_typing = matches!(key.code, KeyCode::Char(_)) && input.should_emit_typing(state.now);

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
            state.nav.focus_ring.set(FocusId::DmList);
            vec![]
        }
        InputBoxResult::TypingActivity => {
            if should_typing {
                let (_, effect) = state.in_flight.track_request(
                    RequestKind::Typing,
                    rekindle_types::daemon::DaemonRequest::Chat(
                        rekindle_types::daemon::ChatRequest::DmTyping { peer_key, typing: true },
                    ),
                    state.now,
                );
                vec![effect]
            } else {
                vec![]
            }
        }
        InputBoxResult::OverLimit(msg) => {
            state.ephemeral.toasts.push(
                msg,
                crate::v2::tui::state::ephemeral::ToastLevel::Warning,
                state.now,
            );
            vec![]
        }
        InputBoxResult::None => vec![],
    }
}

fn selected_index(state: &TuiState) -> Option<usize> {
    state.session.dm_selected_peer.as_deref()
        .and_then(|pk| state.dm.threads.get_index_of(pk))
}

fn set_selected_by_index(state: &mut TuiState, index: usize) {
    state.session.dm_selected_peer = state.dm.threads
        .get_index(index)
        .map(|(k, _)| k.clone());
}

fn load_selected_if_needed(state: &mut TuiState) -> Vec<Effect> {
    let peer_key = match state.session.dm_selected_peer.clone() {
        Some(pk) => pk,
        None => return vec![],
    };
    let thread = match state.dm.threads.get_mut(&peer_key) {
        Some(t) => t,
        None => return vec![],
    };
    if thread.loaded || thread.loading {
        return vec![];
    }
    thread.loading = true;
    let (_, effect) = state.in_flight.track_request(
        RequestKind::DmThread { peer_key: peer_key.clone() },
        rekindle_types::daemon::DaemonRequest::Chat(
            rekindle_types::daemon::ChatRequest::DmThread { peer_key, limit: 50 },
        ),
        state.now,
    );
    vec![effect]
}
