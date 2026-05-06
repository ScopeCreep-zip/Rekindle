//! Confirmation dialog — modal overlay for destructive operations.
//!
//! Centered popup with Clear background, bordered content, prompt text,
//! consequence description, and two buttons. Default focus is on Cancel
//! (safe default). Esc always cancels. Enter confirms the focused button.
//!
//! Accessibility: the Confirm button for destructive operations uses the
//! `error` theme color AND a `[Confirm]` text label. Cancel uses dim
//! styling AND a `[Cancel]` text label. Never color alone.
//!
//! Source patterns:
//! - oxicord `presentation/widgets/confirmation_modal.rs` — Clear + centered_rect
//! - vortix `ui/overlays/confirm_dialog.rs` — generic ConfirmDialogConfig

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::theme::ThemeManager;

/// State for the confirmation dialog.
pub struct ConfirmDialogState {
    /// Prompt text — what are we confirming?
    pub prompt: String,
    /// Consequence description — what happens if confirmed.
    pub consequence: String,
    /// Whether the Confirm button is focused (false = Cancel focused).
    pub confirm_focused: bool,
    /// Whether the dialog is visible.
    pub visible: bool,
}

impl ConfirmDialogState {
    /// Create a new invisible dialog. Call `show()` to display.
    pub fn new() -> Self {
        Self {
            prompt: String::new(),
            consequence: String::new(),
            confirm_focused: false,
            visible: false,
        }
    }

    /// Show the dialog with the given prompt and consequence.
    pub fn show(&mut self, prompt: impl Into<String>, consequence: impl Into<String>) {
        self.prompt = prompt.into();
        self.consequence = consequence.into();
        self.confirm_focused = false; // default to Cancel (safe)
        self.visible = true;
    }

    /// Hide the dialog.
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Toggle button focus between Cancel and Confirm.
    pub fn toggle_focus(&mut self) {
        self.confirm_focused = !self.confirm_focused;
    }

    /// Whether the user confirmed (Confirm button was focused + Enter pressed).
    pub fn is_confirmed(&self) -> bool {
        self.confirm_focused
    }
}

/// Render the confirmation dialog as a centered overlay.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &ConfirmDialogState,
    theme: &ThemeManager,
) {
    if !state.visible {
        return;
    }

    // Calculate popup size — accommodate content
    let popup_width = 52u16.min(area.width.saturating_sub(4));
    let popup_height = 9u16.min(area.height.saturating_sub(2));
    let popup = centered_rect(area, popup_width, popup_height);

    // Clear the background
    frame.render_widget(Clear, popup);

    // Bordered block
    let block = Block::bordered()
        .title(" Confirm ")
        .border_style(Style::default().fg(theme.color("warning")));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Content area split: text + buttons
    let [text_area, _, button_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // Prompt and consequence text
    let text = Paragraph::new(vec![
        Line::from(format!("  {}", state.prompt)),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", state.consequence),
            Style::new().dim(),
        )),
    ])
    .wrap(Wrap { trim: true });
    frame.render_widget(text, text_area);

    // Buttons — Cancel (left) and Confirm (right)
    let cancel_style = if state.confirm_focused {
        Style::new().dim()
    } else {
        Style::default()
            .fg(theme.color("bg.base"))
            .bg(theme.color("accent.primary"))
            .bold()
    };

    let confirm_style = if state.confirm_focused {
        Style::default()
            .fg(theme.color("bg.base"))
            .bg(theme.color("error"))
            .bold()
    } else {
        Style::new().dim()
    };

    let buttons = Line::from(vec![
        Span::raw("  "),
        Span::styled(" [Cancel] ", cancel_style),
        Span::raw("   "),
        Span::styled(" [Confirm] ", confirm_style),
    ]);
    frame.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        button_area,
    );
}

/// Center a rect within a larger area.
fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_invisible() {
        let state = ConfirmDialogState::new();
        assert!(!state.visible);
    }

    #[test]
    fn show_makes_visible_with_cancel_default() {
        let mut state = ConfirmDialogState::new();
        state.show("Delete?", "This cannot be undone.");
        assert!(state.visible);
        assert!(!state.confirm_focused);
        assert!(!state.is_confirmed());
    }

    #[test]
    fn toggle_focus_switches_buttons() {
        let mut state = ConfirmDialogState::new();
        state.show("Delete?", "");
        assert!(!state.confirm_focused);
        state.toggle_focus();
        assert!(state.confirm_focused);
        assert!(state.is_confirmed());
        state.toggle_focus();
        assert!(!state.confirm_focused);
    }

    #[test]
    fn hide_resets_visibility() {
        let mut state = ConfirmDialogState::new();
        state.show("Test", "");
        state.hide();
        assert!(!state.visible);
    }

    #[test]
    fn centered_rect_centers_correctly() {
        let area = Rect::new(0, 0, 80, 24);
        let popup = centered_rect(area, 40, 10);
        assert_eq!(popup.x, 20);
        assert_eq!(popup.y, 7);
        assert_eq!(popup.width, 40);
        assert_eq!(popup.height, 10);
    }

    #[test]
    fn centered_rect_clamps_to_area() {
        let area = Rect::new(0, 0, 20, 8);
        let popup = centered_rect(area, 50, 20);
        assert_eq!(popup.width, 20);
        assert_eq!(popup.height, 8);
    }
}
