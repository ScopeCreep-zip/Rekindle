//! Message input box — tui-textarea wrapper with modes, limits, and styling.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

const MAX_MESSAGE_LENGTH: usize = 2000;

/// Minimum interval between typing indicator emissions.
const TYPING_COOLDOWN: Duration = Duration::from_secs(3);

/// Determines block title and submit behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputMode {
    Compose,
    Reply { message_id: String, author: String },
    Edit { message_id: String },
}

/// Result of `handle_key`. The caller decides what to do with each variant.
pub enum InputBoxResult {
    /// Key was consumed, no action needed.
    None,
    /// User pressed Enter with non-empty, within-limit text. Text extracted.
    Submit(String),
    /// User pressed Esc in Compose mode.
    ExitInputMode,
    /// A character was typed and the typing cooldown has elapsed.
    TypingActivity,
    /// Message exceeds MAX_MESSAGE_LENGTH. Carries the formatted warning.
    OverLimit(String),
}

/// Message input box with ratatui-textarea, mode tracking, and typing cooldown.
#[derive(Debug)]
pub struct InputBox {
    textarea: ratatui_textarea::TextArea<'static>,
    mode: InputMode,
    placeholder: &'static str,
    /// Rate-limited typing indicator emission.
    last_typing_sent: Option<Instant>,
}

impl InputBox {
    pub fn new() -> Self {
        let mut textarea = ratatui_textarea::TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(Style::default());
        textarea.set_max_histories(64);

        Self {
            textarea,
            mode: InputMode::Compose,
            placeholder: "Type a message... (i to focus, Enter to send)",
            last_typing_sent: None,
        }
    }

    pub fn content(&self) -> String {
        self.textarea.lines().join("\n")
    }

    pub fn content_len(&self) -> usize {
        self.textarea.lines().iter().map(String::len).sum::<usize>()
            + self.textarea.lines().len().saturating_sub(1)
    }

    pub fn clear(&mut self) {
        self.textarea.select_all();
        self.textarea.cut();
        self.mode = InputMode::Compose;
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> &InputMode {
        &self.mode
    }

    #[must_use]
    pub fn is_over_limit(&self) -> bool {
        self.content_len() > MAX_MESSAGE_LENGTH
    }

    pub fn insert_text(&mut self, text: &str) {
        self.textarea.insert_str(text);
    }

    /// Returns true if the typing cooldown (3s) has elapsed since the last
    /// emission. Resets the timer on true.
    #[must_use]
    pub fn should_emit_typing(&mut self, now: Instant) -> bool {
        let should = self.last_typing_sent
            .is_none_or(|last| now.duration_since(last) >= TYPING_COOLDOWN);
        if should {
            self.last_typing_sent = Some(now);
        }
        should
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputBoxResult {
        match key.code {
            KeyCode::Esc => {
                if self.mode != InputMode::Compose {
                    self.mode = InputMode::Compose;
                    return InputBoxResult::None;
                }
                InputBoxResult::ExitInputMode
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                let text = self.content();
                if text.trim().is_empty() {
                    return InputBoxResult::None;
                }
                if self.is_over_limit() {
                    return InputBoxResult::OverLimit(format!(
                        "Message too long ({} chars, max {MAX_MESSAGE_LENGTH})",
                        self.content_len(),
                    ));
                }
                InputBoxResult::Submit(text)
            }
            KeyCode::Enter => {
                self.textarea.input(key);
                InputBoxResult::None
            }
            KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.textarea.undo();
                InputBoxResult::None
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.textarea.redo();
                InputBoxResult::None
            }
            _ => {
                if matches!(key.code, KeyCode::Char(_))
                    && self.content_len() >= MAX_MESSAGE_LENGTH
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    return InputBoxResult::None;
                }
                let is_char = matches!(key.code, KeyCode::Char(_));
                self.textarea.input(key);
                if is_char {
                    InputBoxResult::TypingActivity
                } else {
                    InputBoxResult::None
                }
            }
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border_style = if focused { Style::new() } else { Style::new().dim() };
        let block = Block::bordered().title(self.title()).border_style(border_style);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.textarea.lines().iter().all(String::is_empty) && !focused {
            frame.render_widget(
                Paragraph::new(Span::styled(self.placeholder, Style::new().dim().italic())),
                inner,
            );
        } else {
            frame.render_widget(&self.textarea, inner);
        }

        if self.is_over_limit() {
            let warning = Paragraph::new(format!(
                " {}/{MAX_MESSAGE_LENGTH} — too long! ",
                self.content_len(),
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
    }

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
            InputMode::Reply { author, .. } => format!(" Reply to {author} "),
            InputMode::Edit { .. } => " Edit message ".into(),
        }
    }
}

impl Default for InputBox {
    fn default() -> Self {
        Self::new()
    }
}
