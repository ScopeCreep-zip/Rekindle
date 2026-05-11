//! Message list keyboard input handling.

use crossterm::event::KeyCode;

use super::state::MessageList;
use crate::v2::tui::action::Action;
use crate::v2::tui::components::Component;

impl Component for MessageList {
    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => { self.scroll_down(); None }
            KeyCode::Char('k') | KeyCode::Up => { self.scroll_up(); None }
            KeyCode::Char('G') | KeyCode::End => { self.scroll_to_bottom(); None }
            KeyCode::Home => { self.scroll_to_top(); None }
            KeyCode::PageDown => { for _ in 0..10 { self.scroll_down(); } None }
            KeyCode::PageUp => { for _ in 0..10 { self.scroll_up(); } None }
            KeyCode::Char('i') => Some(Action::EnterInputMode),
            KeyCode::Char('r') => Some(Action::ReplyToSelected),
            KeyCode::Char('e') => Some(Action::EditSelected),
            KeyCode::Char('x') | KeyCode::Delete => {
                self.selected_index().and_then(|idx| {
                    self.message_at(idx).map(|msg| Action::DeleteMessage {
                        community: self.community.clone(),
                        channel: self.channel.clone(),
                        message_id: msg.message_id.clone(),
                    })
                })
            }
            KeyCode::Char('y') => {
                self.selected_index().and_then(|idx| {
                    self.message_at(idx).and_then(|msg| {
                        if msg.is_encrypted { None }
                        else { Some(Action::YankToClipboard { text: msg.body.clone() }) }
                    })
                })
            }
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        self.draw_messages(frame, area);
    }

    fn set_focused(&mut self, focused: bool) { self.is_focused = focused; }
}
