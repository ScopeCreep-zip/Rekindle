//! Tab bar rendering — horizontal tabs with scroll indicators and click regions.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::v2::tui::state::tab_bar::TabBarState;
use crate::v2::tui::components::unread_badge;
use crate::v2::tui::theme::ThemeManager;

/// Render the tab bar.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &TabBarState,
    click_regions: &mut Vec<(u16, u16, usize)>,
    theme: &ThemeManager,
) {
    click_regions.clear();
    if state.tabs().is_empty() {
        frame.render_widget(
            Paragraph::new(theme.span("title", " rekindle")),
            area,
        );
        return;
    }

    let mut spans: Vec<Span<'_>> = Vec::new();
    let mut used_width: u16 = 0;
    let max_width = area.width;

    if state.scroll_offset() > 0 {
        spans.push(theme.span("dim", "◀ "));
        used_width += 2;
    }

    let tab_count = state.tabs().len();
    let selected = state.selected();
    let skip = state.scroll_offset();
    let tab_snapshot: Vec<(String, u32)> = state.tabs().iter()
        .map(|t| (t.label.clone(), t.unread))
        .collect();
    let mut visible_end = tab_count;

    for (i, (label, unread)) in tab_snapshot.iter().enumerate().skip(skip) {
        let is_selected = i == selected;

        let badge = unread_badge::unread_span(*unread, theme);
        let badge_text = unread_badge::format_unread(*unread);
        let label_text = format!(" {} ", label);
        #[allow(clippy::cast_possible_truncation)]
        let total_width = label_text.len() as u16 + badge_text.len() as u16;

        if used_width + total_width + 2 > max_width {
            visible_end = i;
            break;
        }

        let tab_start_x = area.x + used_width;
        click_regions.push((tab_start_x, tab_start_x + total_width, i));

        let style = if is_selected {
            theme.mode_normal_style()
        } else {
            theme.style("dim")
        };

        spans.push(Span::styled(label_text, style));
        if *unread > 0 {
            spans.push(badge);
        }
        spans.push(Span::raw("│"));
        used_width += total_width + 1;
    }

    if visible_end < tab_count {
        spans.push(theme.span("dim", " ▶"));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
