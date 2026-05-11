//! Typing indicator — inline annotation and compact summary.

use ratatui::style::Style;
use ratatui::text::Span;

use super::super::theme::ThemeManager;

/// Animated typing suffix for inline display next to a name.
/// Cycles: " ·" → " ··" → " ···" on tick.
#[allow(dead_code)]
pub fn typing_suffix(is_typing: bool, tick_count: u32, theme: &ThemeManager) -> Span<'static> {
    if !is_typing {
        return Span::raw("");
    }
    let dots = match tick_count % 3 {
        0 => " ·",
        1 => " ··",
        _ => " ···",
    };
    Span::styled(dots.to_string(), Style::default().fg(theme.color("text.muted")))
}

/// Compact typing summary for status bar and channel footer.
/// - 0: ""
/// - 1: "alice typing"
/// - 2: "alice, bob typing"
/// - 3+: "alice, bob +1 typing"
pub fn format_typing_compact(typers: &[String]) -> String {
    match typers.len() {
        0 => String::new(),
        1 => format!("{} typing", typers[0]),
        2 => format!("{}, {} typing", typers[0], typers[1]),
        n => format!("{}, {} +{} typing", typers[0], typers[1], n - 2),
    }
}
