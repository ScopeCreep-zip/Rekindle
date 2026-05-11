//! Search overlay rendering — centered modal with input and results.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use super::state::SearchOverlay;
use crate::v2::helpers::sanitize_for_display;
use crate::v2::tui::action::SearchMode;
use crate::v2::tui::theme::ThemeManager;

impl SearchOverlay {
    pub fn render(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        if !self.visible { return; }

        let popup = centered_rect(area, area.width.saturating_sub(8), area.height.saturating_sub(4));
        frame.render_widget(Clear, popup);

        let title = match self.mode {
            SearchMode::QuickSwitch => " Quick Switch (Ctrl+K) ",
            SearchMode::MessageSearch => " Search Messages ",
            SearchMode::CommandPalette => " Command Palette ",
        };

        let block = Block::bordered().title(title).border_style(theme.focused_border());
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let [input_area, results_area] = Layout::vertical([
            Constraint::Length(1), Constraint::Fill(1),
        ]).areas(inner);

        // Search input with cursor
        let input_line = Line::from(vec![
            Span::raw(format!(" > {}", self.query)),
            Span::styled("█", Style::new().dim()),
        ]);
        frame.render_widget(Paragraph::new(input_line), input_area);

        // Results
        if self.filtered_indices.is_empty() {
            let empty = if self.query.is_empty() { "  Type to search..." } else { "  No results." };
            frame.render_widget(Paragraph::new(empty).style(Style::new().dim()), results_area);
            return;
        }

        let items: Vec<ListItem<'_>> = self.filtered_indices.iter().map(|&idx| {
            let item = &self.items[idx];
            let label = sanitize_for_display(&item.label);
            let detail = if item.detail.is_empty() { String::new() }
            else { format!("  ({})", sanitize_for_display(&item.detail)) };

            ListItem::new(Line::from(vec![
                Span::raw(format!("  {label}")),
                Span::styled(detail, Style::new().dim()),
            ]))
        }).collect();

        let count_label = format!(" {}/{} ", self.filtered_indices.len(), self.items.len());
        let list = List::new(items)
            .highlight_style(Style::new().reversed())
            .block(Block::default().title(count_label));

        frame.render_stateful_widget(list, results_area, &mut self.list_state);
    }
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}
