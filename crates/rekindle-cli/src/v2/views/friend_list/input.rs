//! Friend list input — scroll, accept/reject, open DM.

use crossterm::event::KeyCode;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::confirm::PendingConfirmAction;
use crate::v2::tui::state::navigation::{OverlayState, ViewKind};
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    _caches: &mut RenderCaches,
) -> Vec<Effect> {
    let TerminalEvent::Key(key) = event else { return vec![]; };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            move_selection(state, 1);
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            move_selection(state, -1);
            vec![]
        }
        KeyCode::Char('h') => {
            state.nav.pop_view();
            vec![]
        }
        KeyCode::Char('l') | KeyCode::Enter => {
            if let Some(ref pk) = state.session.friend_selected_key {
                if state.friends.friends.iter().any(|f| f.public_key == *pk) {
                    return vec![Effect::Navigate(ViewKind::DmThread { peer_key: pk.clone() })];
                }
            }
            vec![]
        }
        KeyCode::Char('a') => {
            if let Some(req) = selected_pending(state) {
                let (_, effect) = state.in_flight.track_request(
                    crate::v2::tui::state::in_flight::RequestKind::Send,
                    rekindle_types::daemon::DaemonRequest::Chat(
                        rekindle_types::daemon::ChatRequest::FriendAccept { public_key: req },
                    ),
                    state.now,
                );
                return vec![effect];
            }
            vec![]
        }
        KeyCode::Char('r') => {
            if let Some(req) = selected_pending(state) {
                let (_, effect) = state.in_flight.track_request(
                    crate::v2::tui::state::in_flight::RequestKind::Send,
                    rekindle_types::daemon::DaemonRequest::Chat(
                        rekindle_types::daemon::ChatRequest::FriendReject { public_key: req },
                    ),
                    state.now,
                );
                return vec![effect];
            }
            vec![]
        }
        KeyCode::Home => {
            if let Some(first) = state.friends.friends.first() {
                state.session.friend_selected_key = Some(first.public_key.clone());
            }
            vec![]
        }
        KeyCode::End => {
            if let Some(last) = state.friends.friends.last() {
                state.session.friend_selected_key = Some(last.public_key.clone());
            }
            vec![]
        }
        KeyCode::Char('X') => {
            if let Some(ref pk) = state.session.friend_selected_key {
                if let Some(friend) = state.friends.friends.iter().find(|f| f.public_key == *pk) {
                    state.nav.confirm.show(
                        format!("Remove {}?", friend.display_name),
                        "They will be removed from your friend list.",
                        PendingConfirmAction::RemoveFriend {
                            peer_key: pk.clone(),
                        },
                    );
                    state.nav.overlay = Some(OverlayState::Confirm);
                }
            }
            vec![]
        }
        _ => vec![],
    }
}

fn move_selection(state: &mut TuiState, delta: i32) {
    let friends = &state.friends.friends;
    if friends.is_empty() {
        return;
    }
    let current = state.session.friend_selected_key.as_deref()
        .and_then(|pk| friends.iter().position(|f| f.public_key == pk))
        .unwrap_or(0);
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
    let new = if delta > 0 {
        (current + delta as usize).min(friends.len() - 1)
    } else {
        current.saturating_sub((-delta) as usize)
    };
    if let Some(f) = friends.get(new) {
        state.session.friend_selected_key = Some(f.public_key.clone());
    }
}

fn selected_pending(state: &TuiState) -> Option<String> {
    let pk = state.session.friend_selected_key.as_deref()?;
    state.friends.pending_requests.iter()
        .find(|r| r.public_key == pk)
        .map(|r| r.public_key.clone())
}
