//! Status bar — bottom line: mode badge + breadcrumb + typing + node status + hints.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::theme::ThemeManager;

/// Current input mode for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
}

/// Data needed to render the status bar.
pub struct StatusBarState {
    pub mode: Mode,
    pub breadcrumb: String,
    pub typing_context: String,
    pub node_attached: bool,
    pub peer_count: usize,
    pub hints: String,
}

/// Render the status bar.
pub fn render(frame: &mut Frame, area: Rect, state: &StatusBarState, theme: &ThemeManager) {
    let mode_badge = match state.mode {
        Mode::Normal => Span::styled(" [NORMAL] ", theme.mode_normal_style()),
        Mode::Insert => Span::styled(" [INSERT] ", theme.mode_insert_style()),
        Mode::Search => Span::styled(" [SEARCH] ", theme.mode_search_style()),
    };

    let breadcrumb = if state.breadcrumb.is_empty() {
        Span::raw("")
    } else {
        Span::styled(format!(" {} ", state.breadcrumb), theme.style("dim"))
    };

    let typing = if state.typing_context.is_empty() {
        Span::raw("")
    } else {
        Span::styled(
            format!(" {} ", state.typing_context),
            Style::new().fg(theme.color("text.muted")).italic(),
        )
    };

    let node_glyph = theme.status_glyph(state.node_attached);
    let node_text = if state.node_attached {
        format!(" {node_glyph} {} peers ", state.peer_count)
    } else {
        format!(" {node_glyph} offline ")
    };
    let node_span = Span::styled(node_text, theme.style("dim"));

    let separator = Span::raw(" │ ");

    let hints = Span::styled(format!(" {} ", state.hints), theme.style("dim"));

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            mode_badge, breadcrumb, typing,
            separator.clone(), node_span, separator, hints,
        ])),
        area,
    );
}
