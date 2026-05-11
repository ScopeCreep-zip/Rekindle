//! Channel watch layout — responsive 3-pane with split-pane DM support.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use anyhow::Result;

use super::state::{ChannelWatchView, SIDEBAR_COLLAPSE_WIDTH, PEER_LIST_COLLAPSE_WIDTH, SIDEBAR_WIDTH, PEER_LIST_WIDTH};
use crate::v2::tui::components::Component;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::theme::ThemeManager;
use crate::v2::views::View;

impl View for ChannelWatchView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        self.terminal_width = area.width;
        self.update_focus_ring();

        let show_sidebar = self.sidebar_visible && area.width >= SIDEBAR_COLLAPSE_WIDTH;
        let show_peers = area.width >= PEER_LIST_COLLAPSE_WIDTH;

        let mut h_constraints: Vec<Constraint> = Vec::new();
        if show_sidebar { h_constraints.push(Constraint::Length(SIDEBAR_WIDTH)); }
        h_constraints.push(Constraint::Fill(1));
        if show_peers { h_constraints.push(Constraint::Length(PEER_LIST_WIDTH)); }

        let h_areas = Layout::horizontal(h_constraints).split(area);
        let mut col = 0;
        self.click_rects.clear();

        if show_sidebar {
            self.channel_tree.set_focused(self.focus.is_focused(FocusId::ChannelTree));
            self.channel_tree.draw(frame, h_areas[col]);
            self.click_rects.insert(FocusId::ChannelTree, h_areas[col]);
            col += 1;
        }

        let center_area = h_areas[col];
        col += 1;

        if self.split_dm.active {
            let [channel_half, dm_half] = Layout::horizontal([
                Constraint::Percentage(50), Constraint::Percentage(50),
            ]).areas(center_area);
            self.render_channel_pane(frame, channel_half);
            self.render_split_dm_pane(frame, dm_half);
        } else {
            self.render_channel_pane(frame, center_area);
        }

        if show_peers {
            self.peer_list.set_focused(self.focus.is_focused(FocusId::PeerList));
            self.peer_list.draw(frame, h_areas[col]);
            self.click_rects.insert(FocusId::PeerList, h_areas[col]);
        }

        Ok(())
    }

    fn update(&mut self, action: crate::v2::tui::action::Action) -> Result<Option<crate::v2::tui::action::Action>> {
        Ok(super::input::handle_update(self, action))
    }

    fn on_command_result(&mut self, result: crate::v2::tui::action::CommandResult) -> Result<()> {
        super::events::handle_command_result(self, result);
        Ok(())
    }

    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> {
        super::events::handle_subscription_event(self, event);
        Ok(())
    }

    fn tick(&mut self) -> Result<()> {
        self.expire_typing_indicators();
        self.pending_mek_requests.clear();
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<crate::v2::tui::action::Action> {
        super::input::handle_focused_key(self, key)
    }

    fn handle_click(&mut self, column: u16, row: u16) -> Option<crate::v2::tui::action::Action> {
        super::input::handle_click(self, column, row)
    }

    fn focus_ring(&mut self) -> &mut crate::v2::tui::focus::FocusRing { &mut self.focus }
}

impl crate::v2::views::ViewQuery for ChannelWatchView {
    fn typing_names(&self) -> Vec<String> {
        self.typing_names_internal()
    }

    fn message_search_index(&self) -> Vec<(String, String, String)> {
        self.message_list.search_index()
    }
}

impl ChannelWatchView {
    fn render_channel_pane(&mut self, frame: &mut Frame, area: Rect) {
        let typing_height = u16::from(self.typing_display().is_some());
        let [msg_area, typing_area, input_area] = Layout::vertical([
            Constraint::Fill(1), Constraint::Length(typing_height), Constraint::Length(3),
        ]).areas(area);

        self.message_list.set_focused(self.focus.is_focused(FocusId::MessageList));
        self.message_list.draw(frame, msg_area);
        self.click_rects.insert(FocusId::MessageList, msg_area);

        if let Some(typing_text) = self.typing_display() {
            frame.render_widget(
                Paragraph::new(format!("  {typing_text}")).style(Style::new().dim().italic()),
                typing_area,
            );
        }

        self.input_box.set_focused(self.focus.is_focused(FocusId::InputBox));
        self.input_box.draw(frame, input_area);
        self.click_rects.insert(FocusId::InputBox, input_area);
    }

    fn render_split_dm_pane(&mut self, frame: &mut Frame, area: Rect) {
        let typing_height = u16::from(self.split_dm.peer_typing);
        let [msg_area, typing_area, input_area] = Layout::vertical([
            Constraint::Fill(1), Constraint::Length(typing_height), Constraint::Length(3),
        ]).areas(area);

        if let Some(ref mut ml) = self.split_dm_message_list {
            ml.set_focused(self.focus.is_focused(FocusId::SplitDmMessages));
            ml.draw(frame, msg_area);
        }
        self.click_rects.insert(FocusId::SplitDmMessages, msg_area);

        if self.split_dm.peer_typing {
            let text = crate::v2::tui::components::typing_indicator::format_typing_compact(
                std::slice::from_ref(&self.split_dm.peer_name),
            );
            frame.render_widget(Paragraph::new(format!("  {text}")).style(Style::new().dim().italic()), typing_area);
        }

        if let Some(ref mut ib) = self.split_dm_input_box {
            ib.set_focused(self.focus.is_focused(FocusId::SplitDmInput));
            ib.draw(frame, input_area);
        }
        self.click_rects.insert(FocusId::SplitDmInput, input_area);
    }
}
