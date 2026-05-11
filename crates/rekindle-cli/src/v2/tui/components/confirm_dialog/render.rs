//! Confirmation dialog rendering — centered popup with Cancel/Confirm buttons.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::state::ConfirmDialogState;
use crate::v2::tui::theme::ThemeManager;

pub fn render(frame: &mut Frame, area: Rect, state: &ConfirmDialogState, theme: &ThemeManager) {
    if !state.visible { return; }

    let popup_width = 52u16.min(area.width.saturating_sub(4));
    let popup_height = 9u16.min(area.height.saturating_sub(2));
    let popup = centered_rect(area, popup_width, popup_height);

    frame.render_widget(Clear, popup);

    let block = Block::bordered()
        .title(" Confirm ")
        .border_style(Style::default().fg(theme.color("warning")));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [text_area, _, button_area] = Layout::vertical([
        Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1),
    ]).areas(inner);

    let text = Paragraph::new(vec![
        Line::from(format!("  {}", state.prompt)),
        Line::from(""),
        Line::from(Span::styled(format!("  {}", state.consequence), Style::new().dim())),
    ]).wrap(Wrap { trim: true });
    frame.render_widget(text, text_area);

    let cancel_style = if state.confirm_focused {
        Style::new().dim()
    } else {
        Style::new().fg(theme.color("bg.base")).bg(theme.color("accent.primary")).bold()
    };

    let confirm_style = if state.confirm_focused {
        Style::new().fg(theme.color("bg.base")).bg(theme.color("error")).bold()
    } else {
        Style::new().dim()
    };

    let buttons = Line::from(vec![
        Span::raw("  "),
        Span::styled(" [Cancel] ", cancel_style),
        Span::raw("   "),
        Span::styled(" [Confirm] ", confirm_style),
    ]);
    frame.render_widget(Paragraph::new(buttons).alignment(Alignment::Center), button_area);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}
