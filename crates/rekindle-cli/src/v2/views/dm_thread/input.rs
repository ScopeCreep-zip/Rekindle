//! DM thread input handling.

use super::DmThreadView;
use crate::v2::tui::action::Action;
use crate::v2::tui::components::Component;
use crate::v2::tui::focus::FocusId;
use rekindle_types::display::DecryptedMessageDisplay;

pub fn handle_update(view: &mut DmThreadView, action: &Action) -> Option<Action> {
    match action {
        Action::FocusNext => view.focus.next(),
        Action::FocusPrev => view.focus.prev(),
        Action::EnterInputMode => { view.focus.set(FocusId::InputBox); }
        Action::ExitInputMode => { view.focus.set(FocusId::MessageList); }
        Action::InputSubmit => {
            let text = view.input_box.content();
            if !text.trim().is_empty() && !view.input_box.is_over_limit() {
                let now = rekindle_utils::timestamp_ms();
                view.message_list.push(DecryptedMessageDisplay {
                    message_id: format!("pending-{now}"), sequence: 0,
                    author_pseudonym: String::new(), author_display_name: "you".to_string(),
                    body: text.clone(), timestamp: now, reply_to_sequence: None,
                    mek_generation: 0, is_encrypted: false, needs_mek: None,
                    delivery_status: rekindle_types::display::DeliveryStatus::Sending,
                });
                view.input_box.clear();
                return Some(Action::SendDm { peer_key: view.peer_key.clone(), text });
            }
        }
        Action::ScrollDown(n) => { for _ in 0..*n { view.message_list.scroll_down(); } }
        Action::ScrollUp(n) => { for _ in 0..*n { view.message_list.scroll_up(); } }
        Action::ScrollToBottom => view.message_list.scroll_to_bottom(),
        Action::ScrollToTop => view.message_list.scroll_to_top(),
        Action::ScrollToMessage { ref message_id } => {
            view.message_list.scroll_to_message(message_id);
            view.focus.set(FocusId::MessageList);
        }
        Action::FileSelected { ref path } => {
            if view.focus.is_focused(FocusId::InputBox) {
                view.input_box.insert_text(path);
            }
        }
        _ => {}
    }
    None
}

pub fn handle_focused_key(view: &mut DmThreadView, key: crossterm::event::KeyEvent) -> Option<Action> {
    match view.focus.current() {
        FocusId::MessageList => view.message_list.handle_key(key),
        FocusId::InputBox => {
            if key.code == crossterm::event::KeyCode::Char('p')
                && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            {
                return Some(Action::OpenQuickSwitcher);
            }
            if matches!(key.code, crossterm::event::KeyCode::Char(_)) && view.input_box.should_emit_typing() {
                let _ = view.input_box.handle_key(key);
                return Some(Action::SendDmTyping { peer_key: view.peer_key.clone() });
            }
            view.input_box.handle_key(key)
        }
        _ => None,
    }
}

pub fn handle_click(view: &mut DmThreadView, column: u16, row: u16) -> Option<Action> {
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
