//! Community info view — metadata, channels, roles, members.

pub mod events;
pub mod input;
pub mod render;

use ratatui::layout::Rect;
use ratatui::Frame;

use rekindle_types::display::CommunityDetail;

use anyhow::Result;

use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use super::View;

pub struct CommunityInfoView {
    pub(crate) focus: FocusRing,
    pub(crate) community: String,
    pub(crate) detail: Option<CommunityDetail>,
    pub(crate) loading: bool,
    pub(crate) selected_channel: usize,
}

impl CommunityInfoView {
    pub fn new(community: String) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::CommunityInfoPanel]),
            community, detail: None, loading: true, selected_channel: 0,
        }
    }
    pub fn community(&self) -> &str { &self.community }
}

impl super::ViewQuery for CommunityInfoView {}

impl View for CommunityInfoView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        render::draw(self, frame, area, theme);
        Ok(())
    }
    fn update(&mut self, action: Action) -> Result<Option<Action>> { Ok(input::handle_update(self, &action)) }
    fn on_command_result(&mut self, result: CommandResult) -> Result<()> { events::handle_command_result(self, result); Ok(()) }
    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> {
        events::handle_subscription_event(self, event);
        Ok(())
    }
    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> { input::handle_focused_key(self, key) }
    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
