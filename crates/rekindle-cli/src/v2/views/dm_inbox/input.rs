//! DM inbox input handling.

use super::DmInboxView;
use crate::v2::tui::action::Action;
use crate::v2::tui::components::Component;
use crate::v2::tui::focus::FocusId;

pub fn handle_update(view: &mut DmInboxView, action: &Action) -> Option<Action> {
    match action {
        Action::FocusNext => view.focus.next(),
        Action::FocusPrev => view.focus.prev(),
        Action::ScrollDown(_) if view.focus.is_focused(FocusId::DmList) => {
            let max = view.threads.len().saturating_sub(1);
            let i = view.thread_list_state.selected().unwrap_or(0);
            view.thread_list_state.select(Some((i + 1).min(max)));
        }
        Action::ScrollUp(_) if view.focus.is_focused(FocusId::DmList) => {
            let i = view.thread_list_state.selected().unwrap_or(0);
            view.thread_list_state.select(Some(i.saturating_sub(1)));
        }
        Action::Select if view.focus.is_focused(FocusId::DmList) => {
            view.focus.set(FocusId::MessageList);
        }
        Action::InputSubmit => {
            let text = view.input_box.content();
            if let Some(peer_key) = view.selected_peer_key() {
                if !text.trim().is_empty() && !view.input_box.is_over_limit() {
                    let action = Action::SendDm { peer_key: peer_key.to_string(), text };
                    view.input_box.clear();
                    return Some(action);
                }
            }
        }
        Action::EnterInputMode => { view.focus.set(FocusId::InputBox); }
        Action::ExitInputMode => { view.focus.set(FocusId::DmList); }
        Action::FileSelected { ref path } => {
            if view.focus.is_focused(FocusId::InputBox) {
                view.input_box.insert_text(path);
            }
        }
        _ => {}
    }
    None
}

pub fn handle_focused_key(view: &mut DmInboxView, key: crossterm::event::KeyEvent) -> Option<Action> {
    match view.focus.current() {
        FocusId::InputBox => {
            if matches!(key.code, crossterm::event::KeyCode::Char(_)) && view.input_box.should_emit_typing() {
                let peer_key = view.selected_peer_key().map(str::to_string);
                if let Some(pk) = peer_key {
                    let _ = view.input_box.handle_key(key);
                    return Some(Action::SendDmTyping { peer_key: pk });
                }
            }
            view.input_box.handle_key(key)
        }
        _ => None,
    }
}

pub fn handle_click(view: &mut DmInboxView, column: u16, row: u16) -> Option<Action> {
    for (&id, rect) in &view.click_rects {
        if column >= rect.x && column < rect.x + rect.width
            && row >= rect.y && row < rect.y + rect.height
        {
            view.focus.set(id);
            if id == FocusId::InputBox { return Some(Action::EnterInputMode); }
            return None;
        }
    }
    None
}
