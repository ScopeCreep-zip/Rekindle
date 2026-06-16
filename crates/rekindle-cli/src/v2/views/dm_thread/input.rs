//! DM thread input — scroll, compose, send, typing indicators.

use crossterm::event::{KeyCode, KeyEvent};

use crate::v2::tui::components::input_box::{InputBox, InputBoxResult};
use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::dm::DmMessage;
use crate::v2::tui::state::in_flight::{PendingSend, RequestKind};
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    peer_key: &str,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    let TerminalEvent::Key(key) = event else {
        return vec![];
    };

    match state.nav.focus_ring.current() {
        FocusId::MessageList => handle_message_key(key, state, peer_key, caches),
        FocusId::InputBox => handle_input_key(key, state, peer_key),
        _ => vec![],
    }
}

fn handle_message_key(
    key: &KeyEvent,
    state: &mut TuiState,
    peer_key: &str,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match key.code {
        KeyCode::Char('h') => {
            state.nav.pop_view();
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            caches.dm_thread.entry(peer_key.to_string()).or_default().list_state.select_next();
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            caches.dm_thread.entry(peer_key.to_string()).or_default().list_state.select_previous();
            vec![]
        }
        KeyCode::Char('G') | KeyCode::End => {
            caches.dm_thread.entry(peer_key.to_string()).or_default().list_state.select_last();
            vec![]
        }
        KeyCode::Home => {
            caches.dm_thread.entry(peer_key.to_string()).or_default().list_state.select_first();
            vec![]
        }
        KeyCode::Char('x') | KeyCode::Delete => {
            if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                thread.retain_messages(|m| m.delivery_status != rekindle_types::display::DeliveryStatus::Failed);
            }
            vec![]
        }
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

fn handle_input_key(key: &KeyEvent, state: &mut TuiState, peer_key: &str) -> Vec<Effect> {
    let input = state.session.dm_inputs
        .entry(peer_key.to_string())
        .or_insert_with(InputBox::new);

    let should_typing = matches!(key.code, KeyCode::Char(_)) && input.should_emit_typing(state.now);

    match input.handle_key(*key) {
        InputBoxResult::Submit(text) => {
            let (_, effect) = state.in_flight.track_send(
                PendingSend::Dm { peer_key: peer_key.to_string(), body: text.clone() },
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::DmSend { peer_key: peer_key.to_string(), body: text.clone() },
                ),
                state.now,
            );
            if let Some(thread) = state.dm.threads.get_mut(peer_key) {
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
        InputBoxResult::TypingActivity => {
            if should_typing {
                let (_, effect) = state.in_flight.track_request(
                    RequestKind::Typing,
                    rekindle_types::daemon::DaemonRequest::Chat(
                        rekindle_types::daemon::ChatRequest::DmTyping { peer_key: peer_key.to_string(), typing: true },
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
