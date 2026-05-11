//! Toast rendering — top-right stacked with level-colored borders.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::state::NotificationStack;
use crate::v2::tui::action::ToastLevel;
use crate::v2::tui::theme::ThemeManager;

impl NotificationStack {
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        for (i, toast) in self.toasts.iter().enumerate() {
            let width = 40u16.min(area.width.saturating_sub(4));
            let height = 3u16;
            let x = area.right().saturating_sub(width + 2);

            #[allow(clippy::cast_possible_truncation)]
            let y = area.y + 1 + (i as u16) * (height + 1);

            let toast_area = Rect { x, y, width, height };
            if y + height > area.bottom() { break; }

            let (border_color, label) = match toast.level {
                ToastLevel::Info => (theme.color("info"), "[INFO]"),
                ToastLevel::Success => (theme.color("success"), "[OK]"),
                ToastLevel::Warning => (theme.color("warning"), "[WARN]"),
                ToastLevel::Error => (theme.color("error"), "[ERR]"),
            };

            frame.render_widget(Clear, toast_area);
            let block = Block::bordered()
                .title(format!(" {label} "))
                .border_style(Style::default().fg(border_color));
            frame.render_widget(
                Paragraph::new(toast.message.as_str()).block(block).wrap(Wrap { trim: true }),
                toast_area,
            );
        }
    }
}
