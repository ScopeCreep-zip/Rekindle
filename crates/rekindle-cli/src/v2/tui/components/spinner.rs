//! Spinner rendering — braille/ASCII loading indicator.

use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::v2::tui::state::ephemeral::SpinnerState;
use crate::v2::tui::theme::ThemeManager;

impl SpinnerState {
    pub fn render_line(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        if !self.active { return; }
        let glyph = self.glyph();
        let text = if self.label.is_empty() {
            glyph.to_string()
        } else {
            format!("{glyph} {}", self.label)
        };
        frame.render_widget(
            Paragraph::new(theme.span("dim", &text)),
            area,
        );
    }
}
