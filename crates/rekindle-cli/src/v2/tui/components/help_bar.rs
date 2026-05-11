//! Help bar — single-line keybinding hints with critical hints pinned right.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::theme::ThemeManager;

/// Render a single-line help hint bar.
pub fn render(frame: &mut Frame, area: Rect, hints: &[(&str, &str)], _theme: &ThemeManager) {
    if area.width < 10 || hints.is_empty() {
        return;
    }

    let critical_keywords = ["Quit", "Toggle help", "Focus next"];
    let (critical, normal): (Vec<_>, Vec<_>) = hints
        .iter()
        .partition(|(_, desc)| critical_keywords.iter().any(|k| desc.contains(k)));

    let separator = " | ";
    let sep_len = separator.len();

    let critical_parts: Vec<String> = critical.iter().map(|(combo, desc)| format!("{combo} {desc}")).collect();
    let critical_str = critical_parts.join(separator);
    let critical_width = critical_str.len();

    let total_width = area.width as usize;
    let available_for_normal = total_width
        .saturating_sub(critical_width)
        .saturating_sub(if critical_width > 0 { sep_len + 2 } else { 0 });

    let mut normal_parts: Vec<String> = Vec::new();
    let mut used = 0;
    for (combo, desc) in &normal {
        let part = format!("{combo} {desc}");
        let part_len = part.len() + if normal_parts.is_empty() { 0 } else { sep_len };
        if used + part_len > available_for_normal { break; }
        used += part_len;
        normal_parts.push(part);
    }
    let normal_str = normal_parts.join(separator);

    let mut spans: Vec<Span<'_>> = vec![Span::raw(" ")];

    if !normal_str.is_empty() {
        spans.push(Span::styled(normal_str, Style::new().dim()));
    }
    if !critical_str.is_empty() {
        if !normal_parts.is_empty() {
            spans.push(Span::styled(format!(" {separator}"), Style::new().dim()));
        }
        spans.push(Span::styled(critical_str, Style::new().dim().bold()));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
