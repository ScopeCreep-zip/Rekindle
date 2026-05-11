//! DM inbox view — conversation list and thread display.

pub mod events;
pub mod input;
pub mod render;

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use ratatui::Frame;
use std::collections::{HashMap, HashSet};

use rekindle_types::display::DmThreadDisplay;

use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::components::input_box::InputBox;
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use super::View;

pub struct DmInboxView {
    pub(crate) focus: FocusRing,
    pub(crate) threads: Vec<DmThreadDisplay>,
    pub(crate) thread_list_state: ListState,
    pub(crate) input_box: InputBox,
    pub(crate) loaded: bool,
    pub(crate) use_unicode: bool,
    pub(crate) typing_peers: HashSet<String>,
    pub(crate) click_rects: HashMap<FocusId, Rect>,
}

impl DmInboxView {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::DmList, FocusId::MessageList, FocusId::InputBox]),
            threads: Vec::new(), thread_list_state: ListState::default(),
            input_box: InputBox::new(), loaded: false, use_unicode,
            typing_peers: HashSet::new(),
            click_rects: HashMap::new(),
        }
    }

    pub fn selected_peer_key(&self) -> Option<&str> {
        self.thread_list_state.selected()
            .and_then(|i| self.threads.get(i))
            .map(|t| t.peer_key.as_str())
    }
}

impl super::ViewQuery for DmInboxView {}

impl View for DmInboxView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> { render::draw(self, frame, area, theme); Ok(()) }
    fn update(&mut self, action: Action) -> Result<Option<Action>> { Ok(input::handle_update(self, &action)) }
    fn on_command_result(&mut self, result: CommandResult) -> Result<()> { events::handle_command_result(self, result); Ok(()) }
    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> {
        events::handle_subscription_event(self, event);
        Ok(())
    }
    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> { input::handle_focused_key(self, key) }
    fn handle_click(&mut self, column: u16, row: u16) -> Option<Action> { input::handle_click(self, column, row) }
    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
