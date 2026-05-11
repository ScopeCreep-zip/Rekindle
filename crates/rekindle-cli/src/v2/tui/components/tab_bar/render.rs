//! Tab bar rendering — horizontal tabs with scroll indicators and click regions.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::state::TabBarState;
use crate::v2::tui::components::unread_badge;
use crate::v2::tui::theme::ThemeManager;

/// Render the tab bar.
pub fn render(frame: &mut Frame, area: Rect, state: &mut TabBarState, theme: &ThemeManager) {
    state.click_regions.clear();
    if state.tabs.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" rekindle", theme.style("title"))),
            area,
        );
        return;
    }

    let mut spans: Vec<Span<'_>> = Vec::new();
    let mut used_width: u16 = 0;
    let max_width = area.width;

    if state.scroll_offset() > 0 {
        spans.push(Span::styled("◀ ", theme.style("dim")));
        used_width += 2;
    }

    let mut visible_end = state.tabs.len();

    for (i, tab) in state.tabs.iter().enumerate().skip(state.scroll_offset()) {
        let is_selected = i == state.selected;

        let badge = unread_badge::unread_span(tab.unread, theme);
        let badge_text = unread_badge::format_unread(tab.unread);
        let label_text = format!(" {} ", tab.label);
        #[allow(clippy::cast_possible_truncation)]
        let total_width = label_text.len() as u16 + badge_text.len() as u16;

        if used_width + total_width + 2 > max_width {
            visible_end = i;
            break;
        }

        let tab_start_x = area.x + used_width;
        state.click_regions.push((tab_start_x, tab_start_x + total_width, i));

        let style = if is_selected {
            theme.mode_normal_style()
        } else {
            theme.style("dim")
        };

        spans.push(Span::styled(label_text, style));
        if tab.unread > 0 {
            spans.push(badge);
        }
        spans.push(Span::raw("│"));
        used_width += total_width + 1;
    }

    if visible_end < state.tabs.len() {
        spans.push(Span::styled(" ▶", theme.style("dim")));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
