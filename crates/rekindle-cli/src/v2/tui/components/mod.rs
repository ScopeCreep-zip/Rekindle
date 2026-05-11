//! TUI component system — reusable UI building blocks.
//!
//! Every interactive visual element implements [`Component`].
//! Components emit [`Action`] variants — they never call async code directly.

pub mod channel_tree;
pub mod confirm_dialog;
pub mod file_content_search;
pub mod help_bar;
pub mod input_box;
pub mod message_list;
pub mod peer_list;
pub mod search_overlay;
pub mod spinner;
pub mod status_bar;
pub mod tab_bar;
pub mod notification_rail;
pub mod toast;
pub mod typing_indicator;
pub mod unread_badge;

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use super::action::Action;

/// Trait for interactive TUI components owned by views.
pub trait Component {
    /// Render the component into the given area.
    fn draw(&mut self, frame: &mut Frame, area: Rect);
    /// Set the focus state.
    fn set_focused(&mut self, focused: bool);
    /// Handle a keyboard input when focused.
    fn handle_key(&mut self, key: KeyEvent) -> Option<Action>;
}
