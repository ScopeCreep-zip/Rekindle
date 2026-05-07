//! Unread badge — reusable formatted count for tabs, channels, and DM threads.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::tui::theme::ThemeManager;

/// Format an unread count for display.
///
/// Returns "" for 0, "(N)" for 1-99, "(99+)" for 100+.
pub fn format_unread(count: u32) -> String {
    match count {
        0 => String::new(),
        1..=99 => format!("({count})"),
        _ => "(99+)".to_string(),
    }
}

/// Create a styled unread badge Span using the theme's accent.secondary color.
/// Wired in M1 when tab_bar and channel_tree pass theme to render methods.
#[allow(dead_code)]
pub fn unread_span(count: u32, theme: &ThemeManager) -> Span<'static> {
    let text = format_unread(count);
    if text.is_empty() {
        Span::raw("")
    } else {
        Span::styled(text, Style::default().fg(theme.color("accent.secondary")).bold())
    }
}
