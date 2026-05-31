//! Message input box — tui-textarea wrapper.
//!
//! Wraps [`ratatui_textarea::TextArea`] with rekindle-specific behavior:
//! - Mode-colored border (accent when focused, dim when not)
//! - Placeholder text when empty and unfocused
//! - `Enter` (no modifiers) = submit, `Shift+Enter` = newline
//! - `Esc` = exit input mode
//! - Max 2000 character limit with visual indicator
//! - Reply/edit mode indicator in the block title
//!
//! Source patterns:
//! - oxicord `presentation/widgets/message_input.rs` — TextArea + mode enum
//! - siggy `ui/composer.rs` — mode-colored border, placeholder

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::Component;
use crate::tui::action::Action;

/// Maximum message length in characters.
const MAX_MESSAGE_LENGTH: usize = 2000;

/// Input mode — determines the block title and submit behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    /// Normal compose mode.
    Compose,
    /// Replying to a specific message.
    Reply { message_id: String, author: String },
    /// Editing an existing message.
    Edit { message_id: String },
}

/// Message input box component.
pub struct InputBox {
    /// The underlying text area state.
    textarea: ratatui_textarea::TextArea<'static>,
    /// Whether this component is currently focused.
    is_focused: bool,
    /// Current input mode (compose, reply, edit).
    mode: InputMode,
    /// Placeholder text shown when empty and unfocused.
    placeholder: &'static str,
}

impl InputBox {
    /// Create a new input box.
    pub fn new() -> Self {
        let mut textarea = ratatui_textarea::TextArea::default();
        // Disable the default line highlight — we use our own border styling
        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(Style::default());
        // Set a reasonable max history depth for undo
        textarea.set_max_histories(64);

        Self {
            textarea,
            is_focused: false,
            mode: InputMode::Compose,
            placeholder: "Type a message... (i to focus, Enter to send)",
        }
    }

    /// Get the current text content.
    pub fn content(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Get the current content length in characters.
    pub fn content_len(&self) -> usize {
        self.textarea.lines().iter().map(String::len).sum::<usize>()
            + self.textarea.lines().len().saturating_sub(1) // newlines
    }

    /// Clear the input box content.
    pub fn clear(&mut self) {
        self.textarea.select_all();
        self.textarea.cut();
        self.mode = InputMode::Compose;
    }

    /// Set the input mode.
    pub fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
    }

    /// Get the current input mode.
    pub fn mode(&self) -> &InputMode {
        &self.mode
    }

    /// Whether the content exceeds the maximum length.
    pub fn is_over_limit(&self) -> bool {
        self.content_len() > MAX_MESSAGE_LENGTH
    }

    /// Build the block title based on mode.
    fn title(&self) -> String {
        match &self.mode {
            InputMode::Compose => {
                let len = self.content_len();
                if len > 0 {
                    format!(" Compose ({len}/{MAX_MESSAGE_LENGTH}) ")
                } else {
                    " Compose ".into()
                }
            }
            InputMode::Reply { author, .. } => {
                format!(" Reply to {author} ")
            }
            InputMode::Edit { .. } => " Edit message ".into(),
        }
    }
}

impl Component for InputBox {
    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if !self.is_focused {
            return None;
        }

        match key.code {
            // Esc exits input mode
            KeyCode::Esc => {
                if self.mode != InputMode::Compose {
                    // If replying/editing, cancel back to compose mode
                    self.mode = InputMode::Compose;
                    return None;
                }
                Some(Action::ExitInputMode)
            }

            // Enter (no modifiers) submits
            KeyCode::Enter if key.modifiers.is_empty() => {
                let text = self.content();
                if text.trim().is_empty() {
                    return None;
                }
                if self.is_over_limit() {
                    return Some(Action::ShowToast {
                        message: format!(
                            "Message too long ({} chars, max {MAX_MESSAGE_LENGTH})",
                            self.content_len()
                        ),
                        level: crate::tui::action::ToastLevel::Warning,
                    });
                }
                Some(Action::InputSubmit)
            }

            // Shift+Enter or Alt+Enter inserts a newline
            KeyCode::Enter => {
                self.textarea.input(key);
                None
            }

            // All other keys go to the textarea
            _ => {
                // Check length limit before inserting
                if matches!(key.code, KeyCode::Char(_))
                    && self.content_len() >= MAX_MESSAGE_LENGTH
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    // At limit — refuse input
                    return None;
                }
                self.textarea.input(key);
                None
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> anyhow::Result<()> {
        let border_style = if self.is_focused {
            Style::new()
        } else {
            Style::new().dim()
        };

        let title = self.title();
        let block = Block::bordered().title(title).border_style(border_style);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Show placeholder when empty and not focused
        if self.textarea.lines().iter().all(String::is_empty) && !self.is_focused {
            let placeholder =
                Paragraph::new(Span::styled(self.placeholder, Style::new().dim().italic()));
            frame.render_widget(placeholder, inner);
        } else {
            frame.render_widget(&self.textarea, inner);
        }

        // Show cursor only when focused.
        // ratatui-textarea renders its own cursor via the Widget impl
        // when the TextArea is rendered. No manual set_cursor_position needed.

        // Over-limit indicator
        if self.is_over_limit() {
            let warning = Paragraph::new(format!(
                " {}/{MAX_MESSAGE_LENGTH} — too long! ",
                self.content_len()
            ))
            .style(Style::new().bold());
            let warning_area = Rect {
                x: area.x + 1,
                y: area.bottom().saturating_sub(1),
                width: area.width.saturating_sub(2),
                height: 1,
            };
            frame.render_widget(warning, warning_area);
        }

        Ok(())
    }

    fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let input = InputBox::new();
        assert!(input.content().is_empty());
        assert_eq!(input.content_len(), 0);
        assert!(!input.is_over_limit());
    }

    #[test]
    fn default_mode_is_compose() {
        let input = InputBox::new();
        assert_eq!(*input.mode(), InputMode::Compose);
    }

    #[test]
    fn set_mode_reply() {
        let mut input = InputBox::new();
        input.set_mode(InputMode::Reply {
            message_id: "msg-1".into(),
            author: "alice".into(),
        });
        assert!(matches!(input.mode(), InputMode::Reply { .. }));
    }

    #[test]
    fn clear_resets_to_compose() {
        let mut input = InputBox::new();
        input.set_mode(InputMode::Edit {
            message_id: "msg-1".into(),
        });
        input.clear();
        assert_eq!(*input.mode(), InputMode::Compose);
    }

    #[test]
    fn title_shows_length_when_composing() {
        let input = InputBox::new();
        let title = input.title();
        assert!(title.contains("Compose"));
    }

    #[test]
    fn title_shows_reply_target() {
        let mut input = InputBox::new();
        input.set_mode(InputMode::Reply {
            message_id: "msg-1".into(),
            author: "alice".into(),
        });
        assert!(input.title().contains("alice"));
    }

    #[test]
    fn starts_unfocused() {
        let input = InputBox::new();
        assert!(!input.is_focused);
    }

    #[test]
    fn focus_set_and_get() {
        let mut input = InputBox::new();
        input.set_focused(true);
        assert!(input.is_focused);
        input.set_focused(false);
        assert!(!input.is_focused);
    }
}
