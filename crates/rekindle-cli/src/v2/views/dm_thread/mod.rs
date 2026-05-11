//! DM thread view — standalone message thread with a single peer.

pub mod events;
pub mod input;
pub mod render;

use ratatui::layout::Rect;
use ratatui::Frame;
use std::collections::HashMap;

use crate::v2::helpers;
use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::components::input_box::InputBox;
use crate::v2::tui::components::message_list::MessageList;
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use super::{View, ViewQuery};

pub struct DmThreadView {
    pub(crate) peer_key: String,
    pub(crate) peer_name: String,
    pub(crate) message_list: MessageList,
    pub(crate) input_box: InputBox,
    pub(crate) focus: FocusRing,
    pub(crate) is_peer_typing: bool,
    pub(crate) loaded: bool,
    #[allow(dead_code)]
    pub(crate) use_unicode: bool,
    pub(crate) click_rects: HashMap<FocusId, Rect>,
}

impl DmThreadView {
    pub fn new(peer_key: String, use_unicode: bool) -> Self {
        let peer_name = helpers::abbreviate_key(&peer_key);
        Self {
            peer_key: peer_key.clone(), peer_name,
            message_list: MessageList::new(String::new(), peer_key),
            input_box: InputBox::new(),
            focus: FocusRing::new(vec![FocusId::MessageList, FocusId::InputBox]),
            is_peer_typing: false, loaded: false, use_unicode,
            click_rects: HashMap::new(),
        }
    }

    pub fn peer_key(&self) -> &str { &self.peer_key }
}

impl ViewQuery for DmThreadView {
    fn typing_names(&self) -> Vec<String> {
        if self.is_peer_typing { vec![self.peer_name.clone()] } else { Vec::new() }
    }

    fn message_search_index(&self) -> Vec<(String, String, String)> {
        self.message_list.search_index()
    }
}

impl View for DmThreadView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> anyhow::Result<()> { render::draw(self, frame, area, theme); Ok(()) }
    fn update(&mut self, action: Action) -> anyhow::Result<Option<Action>> { Ok(input::handle_update(self, &action)) }
    fn on_command_result(&mut self, result: CommandResult) -> anyhow::Result<()> { events::handle_command_result(self, result); Ok(()) }
    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> anyhow::Result<()> {
        events::handle_subscription_event(self, event);
        Ok(())
    }
    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> { input::handle_focused_key(self, key) }
    fn handle_click(&mut self, column: u16, row: u16) -> Option<Action> { input::handle_click(self, column, row) }
    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
