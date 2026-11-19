//! Status bar component — bottom line of the TUI.
//!
//! Displays three sections:
//! - Left: mode badge `[NORMAL]` or `[INSERT]` with distinct styling
//! - Center: breadcrumb path (Community / #channel)
//! - Right: node status indicator with glyph + text + color
//!
//! The mode badge uses BOTH a text label AND a distinct background color
//! so the user always knows their mode, even without color.
//!
//! Source patterns:
//! - siggy `ui/status_bar.rs` — `[NORMAL]`/`[INSERT]` mode indicator
//! - vortix `ui/widgets/footer.rs` — two-tier hint system

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme::ThemeManager;

/// Current input mode for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
}

/// Data needed to render the status bar.
pub struct StatusBarState {
    /// Current input mode.
    pub mode: Mode,
    /// Breadcrumb path — e.g., "dev-team / #general".
    pub breadcrumb: String,
    /// Whether the node is attached to the network.
    pub node_attached: bool,
    /// Veilid's attachment strength as the Host smoothed it, 0-5
    /// (`NetworkEvent::Strength`, plan E7.2): the GUI's bars.
    pub bars: u8,
    /// Keybinding hint string.
    pub hints: String,
}

/// Render the status bar into a single-line area.
pub fn render(frame: &mut Frame, area: Rect, state: &StatusBarState, theme: &ThemeManager) {
    let mode_badge = match state.mode {
        Mode::Normal => Span::styled(
            " [NORMAL] ",
            Style::default()
                .fg(theme.color("bg.base"))
                .bg(theme.color("accent.primary"))
                .bold(),
        ),
        Mode::Insert => Span::styled(
            " [INSERT] ",
            Style::default()
                .fg(theme.color("bg.base"))
                .bg(theme.color("success"))
                .bold(),
        ),
        Mode::Search => Span::styled(
            " [SEARCH] ",
            Style::default()
                .fg(theme.color("bg.base"))
                .bg(theme.color("warning"))
                .bold(),
        ),
    };

    let breadcrumb = if state.breadcrumb.is_empty() {
        Span::raw("")
    } else {
        Span::styled(format!(" {} ", state.breadcrumb), Style::new().dim())
    };

    let node_glyph = theme.status_glyph(state.node_attached);
    let node_text = if state.node_attached {
        format!(
            " {node_glyph} {} ",
            bars_text(state.bars, theme.use_unicode())
        )
    } else {
        format!(" {node_glyph} offline ")
    };
    let node_span = Span::styled(node_text, Style::new().dim());

    let separator = Span::raw(" │ ");

    let hints = Span::styled(format!(" {} ", state.hints), Style::new().dim());

    let line = Line::from(vec![
        mode_badge,
        breadcrumb,
        separator.clone(),
        node_span,
        separator,
        hints,
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

/// The indicator's five bars, rising, lit up to `bars`.
fn bars_text(bars: u8, unicode: bool) -> String {
    const LIT: [char; 5] = ['▁', '▂', '▄', '▆', '█'];
    (1..=5u8)
        .map(|level| match (bars >= level, unicode) {
            (true, true) => LIT[usize::from(level - 1)],
            (false, true) => '·',
            (true, false) => '|',
            (false, false) => '.',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::bars_text;

    /// The TUI shows the Host's bars as the GUI does (plan E7.2).
    #[test]
    fn the_bars_light_up_to_the_hosts_strength() {
        assert_eq!(bars_text(3, true), "▁▂▄··");
        assert_eq!(bars_text(0, false), ".....");
        assert_eq!(bars_text(5, false), "|||||");
    }
}
