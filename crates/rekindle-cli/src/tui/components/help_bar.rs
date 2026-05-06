//! Help bar — single-line keybinding hints.
//!
//! Renders a compact line of context-relevant keybinding hints from the
//! keymap store. Truncates gracefully when the terminal is narrow.
//! Critical hints (q quit, ? help, Tab focus) are always visible at the
//! right edge.
//!
//! Source pattern:
//! - vortix `ui/widgets/footer.rs` — two-tier hint system with critical hints pinned right

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::theme::ThemeManager;

/// Render a single-line help hint bar.
///
/// `hints` is a list of `(key_combo_display, description)` pairs from
/// the keymap store. The bar renders as many as fit, with critical
/// hints pinned to the right edge.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    hints: &[(&str, &str)],
    _theme: &ThemeManager,
) {
    if area.width < 10 || hints.is_empty() {
        return;
    }

    // Partition into critical (always shown) and normal (truncatable).
    // Critical hints: quit, help, focus — identified by description keywords.
    let critical_keywords = ["Quit", "Toggle help", "Focus next"];
    let (critical, normal): (Vec<_>, Vec<_>) = hints
        .iter()
        .partition(|(_, desc)| critical_keywords.iter().any(|k| desc.contains(k)));

    let separator = " | ";
    let sep_len = separator.len();

    // Build critical section first to know how much space is reserved.
    let critical_parts: Vec<String> = critical
        .iter()
        .map(|(combo, desc)| format!("{combo} {desc}"))
        .collect();
    let critical_str = critical_parts.join(separator);
    let critical_width = critical_str.len();

    // Available width for normal hints
    let total_width = area.width as usize;
    let available_for_normal = total_width
        .saturating_sub(critical_width)
        .saturating_sub(if critical_width > 0 { sep_len + 2 } else { 0 });

    // Build normal section, truncating to fit
    let mut normal_parts: Vec<String> = Vec::new();
    let mut used = 0;
    for (combo, desc) in &normal {
        let part = format!("{combo} {desc}");
        let part_len = part.len() + if normal_parts.is_empty() { 0 } else { sep_len };
        if used + part_len > available_for_normal {
            break;
        }
        used += part_len;
        normal_parts.push(part);
    }
    let normal_str = normal_parts.join(separator);

    // Compose the full line
    let mut spans: Vec<Span<'_>> = Vec::new();
    spans.push(Span::raw(" "));

    if !normal_str.is_empty() {
        spans.push(Span::styled(normal_str, Style::new().dim()));
    }

    if !critical_str.is_empty() {
        if !normal_parts.is_empty() {
            spans.push(Span::styled(format!(" {separator}"), Style::new().dim()));
        }
        spans.push(Span::styled(critical_str, Style::new().dim().bold()));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}
