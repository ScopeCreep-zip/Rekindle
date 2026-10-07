//! TUI component system — reusable UI building blocks.
//!
//! Every interactive visual element implements [`Component`].
//! Components emit [`Action`] variants — they never call async code directly.

pub mod channel_tree;
pub mod confirm_dialog;
pub mod help_bar;
pub mod input_box;
pub mod message_list;
pub mod peer_list;
pub mod search_overlay;
pub mod spinner;
pub mod status_bar;
pub mod tab_bar;
pub mod toast;

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use super::action::Action;

/// Trait for interactive TUI components owned by views.
pub trait Component {
    /// Render the component into the given area.
    fn draw(&mut self, frame: &mut Frame, area: Rect) -> anyhow::Result<()>;

    /// Set the focus state. Components change border styles based on focus.
    fn set_focused(&mut self, focused: bool);

    /// Handle a keyboard input event when focused.
    /// Return `Some(Action)` to dispatch to the App reducer.
    /// Called by `View::handle_focused_key()` for the currently focused component.
    fn handle_key(&mut self, key: KeyEvent) -> Option<Action>;
}
