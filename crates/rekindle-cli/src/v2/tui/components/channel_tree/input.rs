//! Channel tree keyboard input handling.

use crossterm::event::{KeyCode, KeyEvent};

use super::tree::ChannelTree;
use super::types::TreeNodeId;
use crate::v2::tui::action::Action;
use crate::v2::tui::components::Component;

impl Component for ChannelTree {
    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.nodes.len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => {
                let selected = self.selected_id().cloned();
                match selected {
                    Some(TreeNodeId::Channel { community, channel }) => {
                        Some(Action::ShowChannel { community, channel })
                    }
                    Some(TreeNodeId::DmUser(peer_key)) => {
                        Some(Action::ShowDmThread { peer_key })
                    }
                    Some(id) if self.nodes.iter().any(|n| n.id == id && n.has_children) => {
                        self.toggle_expand();
                        None
                    }
                    _ => None,
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.collapse_or_parent();
                None
            }
            KeyCode::Home => {
                if !self.nodes.is_empty() { self.list_state.select(Some(0)); }
                None
            }
            KeyCode::End => {
                if !self.nodes.is_empty() { self.list_state.select(Some(self.nodes.len() - 1)); }
                None
            }
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        self.draw_tree(frame, area);
    }

    fn set_focused(&mut self, focused: bool) { self.is_focused = focused; }
}
