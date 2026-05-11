//! Friend list view — friends grouped by presence with pending requests.

pub mod events;
pub mod input;
pub mod render;

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use ratatui::Frame;

use rekindle_types::display::FriendDisplay;

use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use super::View;

#[derive(Debug, Clone)]
pub struct PendingRequestDisplay {
    pub public_key: String,
    pub display_name: String,
    pub message: String,
}

pub struct FriendListView {
    pub(crate) focus: FocusRing,
    pub(crate) friends: Vec<FriendDisplay>,
    pub(crate) list_state: ListState,
    pub(crate) pending_requests: Vec<PendingRequestDisplay>,
    pub(crate) use_unicode: bool,
    pub(crate) loaded: bool,
}

impl FriendListView {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::FriendList]),
            friends: Vec::new(), list_state: ListState::default(),
            pending_requests: Vec::new(), use_unicode, loaded: false,
        }
    }
}

impl super::ViewQuery for FriendListView {}

impl View for FriendListView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> { render::draw(self, frame, area, theme); Ok(()) }
    fn update(&mut self, action: Action) -> Result<Option<Action>> { Ok(input::handle_update(self, &action)) }
    fn on_command_result(&mut self, result: CommandResult) -> Result<()> { events::handle_command_result(self, result); Ok(()) }
    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> {
        events::handle_subscription_event(self, event);
        Ok(())
    }
    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> { input::handle_focused_key(self, key) }
    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}

pub fn presence_rank(status: &str) -> u8 {
    match status { "online" => 0, "away" => 1, "busy" => 2, "offline" => 3, _ => 4 }
}
