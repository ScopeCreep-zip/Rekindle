//! Typing indicator — inline annotation for peer/friend list entries.
//!
//! Typing state is rendered as a suffix on the person's row in the
//! peer list (communities) or friend list (DMs), colocated with their
//! name and presence dot. No additional screen space consumed.
//!
//! Design: the indicator appears where the eye already is — next to
//! the person's name — rather than in a separate status line that
//! fragments attention across the layout.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::tui::theme::ThemeManager;

/// Returns a styled typing suffix span for inline display next to a name.
///
/// Returns an empty span when not typing. When typing, returns an
/// animated ellipsis that cycles on tick: " ·" → " ··" → " ···" → " ·"
///
/// Usage in peer_list or friend_list row rendering:
/// ```ignore
/// let name_span = Span::styled(&entry.display_name, name_style);
/// let typing = typing_suffix(is_typing, tick, theme);
/// Line::from(vec![dot, name_span, typing])
/// ```
#[allow(dead_code)] // Wired when peer_list renders inline typing dots per member
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

/// Format a compact typing summary for contexts where multiple typers
/// need a single-line representation (e.g., status bar, channel footer).
///
/// Returns empty string when nobody is typing.
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
