//! DM thread rendering.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::DmThreadView;
use crate::v2::tui::components::Component;
use crate::v2::tui::components::typing_indicator;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::theme::ThemeManager;

pub fn draw(view: &mut DmThreadView, frame: &mut Frame, area: Rect, _theme: &ThemeManager) {
    let typing_height = u16::from(view.is_peer_typing);
    let [msg_area, typing_area, input_area] = Layout::vertical([
        Constraint::Fill(1), Constraint::Length(typing_height), Constraint::Length(3),
    ]).areas(area);

    view.click_rects.clear();
    view.click_rects.insert(FocusId::MessageList, msg_area);
    view.click_rects.insert(FocusId::InputBox, input_area);

    view.message_list.set_focused(view.focus.is_focused(FocusId::MessageList));
    view.message_list.draw(frame, msg_area);

    if view.is_peer_typing {
        let text = typing_indicator::format_typing_compact(std::slice::from_ref(&view.peer_name));
        frame.render_widget(Paragraph::new(format!("  {text}")).style(Style::new().dim().italic()), typing_area);
    }

    view.input_box.set_focused(view.focus.is_focused(FocusId::InputBox));
    view.input_box.draw(frame, input_area);
}
