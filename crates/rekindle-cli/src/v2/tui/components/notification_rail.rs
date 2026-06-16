//! Notification rail rendering — scope-partitioned signal surfaces.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::v2::tui::state::ephemeral::{NotificationRails, RailSignal, SignalPriority};
use crate::v2::tui::theme::ThemeManager;

impl NotificationRails {
    /// Render the community/channel rail (top position, below tab bar).
    pub fn render_community_rail(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let signals: Vec<&RailSignal> = self.community_signals();
        if signals.is_empty() {
            return;
        }
        let spans = build_rail_spans(&signals, theme);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Render the system rail (bottom position, above status bar).
    pub fn render_system_rail(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let signals: Vec<&RailSignal> = self.system_signals();
        if signals.is_empty() {
            return;
        }
        let spans = build_rail_spans(&signals, theme);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

fn build_rail_spans<'a>(signals: &[&RailSignal], theme: &'a ThemeManager) -> Vec<Span<'a>> {
    let mut sorted: Vec<&&RailSignal> = signals.iter().collect();
    sorted.sort_by(|a, b| b.priority.cmp(&a.priority));

    let mut spans = Vec::new();
    for (i, signal) in sorted.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" \u{2502} ", Style::new().dim()));
        }

        let (icon, style) = match signal.priority {
            SignalPriority::Critical => ("\u{1f534} ", Style::new().fg(theme.color("error")).bold()),
            SignalPriority::Warning => ("\u{26a0} ", Style::new().fg(theme.color("warning"))),
            SignalPriority::Info => ("\u{2139} ", Style::new().fg(theme.color("info"))),
        };

        spans.push(Span::raw(" "));
        spans.push(Span::styled(icon, style));
        spans.push(Span::styled(signal.text.clone(), style));
    }

    spans
}
