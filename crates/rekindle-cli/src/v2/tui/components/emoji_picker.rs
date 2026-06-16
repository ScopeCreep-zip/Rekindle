//! Emoji picker rendering — grid popup with search and selection.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::v2::tui::state::emoji_picker::EmojiPickerState;

impl EmojiPickerState {
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        if !self.visible { return; }

        let width = 34u16;
        let entries = self.filtered_entries();
        let cols = self.cols();
        let rows = ((entries.len() + cols - 1) / cols) as u16;
        let height = (rows + 4).min(12);
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

        frame.render_widget(Clear, popup);
        let block = Block::bordered().title(" React \u{2014} arrows/type, Enter select, Esc close ");

        let mut lines: Vec<Line<'_>> = Vec::new();

        lines.push(Line::from(vec![
            Span::raw(" search: "),
            Span::styled(
                if self.search.is_empty() { "..." } else { &self.search },
                Style::new().dim(),
            ),
        ]));

        for row_start in (0..entries.len()).step_by(cols) {
            let row_end = (row_start + cols).min(entries.len());
            let spans: Vec<Span<'_>> = entries[row_start..row_end].iter().map(|(emoji, _, selected)| {
                let style = if *selected { Style::new().reversed() } else { Style::new() };
                Span::styled(format!(" {emoji} "), style)
            }).collect();
            lines.push(Line::from(spans));
        }

        if entries.is_empty() {
            lines.push(Line::from(Span::styled(" no match", Style::new().dim())));
        }

        frame.render_widget(Paragraph::new(lines).block(block), popup);
    }
}
