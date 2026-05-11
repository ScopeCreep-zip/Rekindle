//! Peer list keyboard input — scroll, click-to-DM split pane.

use crossterm::event::KeyCode;

use super::state::PeerList;
use crate::v2::tui::action::Action;
use crate::v2::tui::components::Component;

impl Component for PeerList {
    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.members.len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                // Open split-pane DM with the selected member
                let visual_idx = self.list_state.selected()?;
                // Visual list includes section headers — map to member index
                let member = visual_to_member(&self.members, visual_idx)?;
                Some(Action::OpenSplitDm { peer_key: member.key.clone() })
            }
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        self.draw_list(frame, area);
    }

    fn set_focused(&mut self, focused: bool) { self.is_focused = focused; }
}

/// Map a visual list index to a member, skipping section headers.
fn visual_to_member(members: &[super::state::PeerEntry], visual_idx: usize) -> Option<&super::state::PeerEntry> {
    let mut current_status: Option<&str> = None;
    let mut visual = 0usize;

    for member in members {
        if current_status != Some(member.status.as_str()) {
            current_status = Some(member.status.as_str());
            if visual == visual_idx { return None; } // header
            visual += 1;
        }
        if visual == visual_idx { return Some(member); }
        visual += 1;
    }
    None
}
