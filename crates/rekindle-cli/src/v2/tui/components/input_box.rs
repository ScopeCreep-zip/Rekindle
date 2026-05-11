//! Message input box — tui-textarea wrapper with modes, limits, and styling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::Component;
use super::super::action::{Action, ToastLevel};

const MAX_MESSAGE_LENGTH: usize = 2000;

/// Typing indicator rate limit — minimum interval between auto-send.
const TYPING_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(3);

/// Input mode — determines block title and submit behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Compose,
    Reply { message_id: String, author: String },
    Edit { message_id: String },
}

/// Message input box component.
pub struct InputBox {
    textarea: ratatui_textarea::TextArea<'static>,
    is_focused: bool,
    mode: InputMode,
    placeholder: &'static str,
    /// Last time a typing indicator was emitted. Rate-limited to TYPING_COOLDOWN.
    last_typing_sent: Option<std::time::Instant>,
}

impl InputBox {
    pub fn new() -> Self {
        let mut textarea = ratatui_textarea::TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(Style::default());
        textarea.set_max_histories(64);

        Self {
            textarea,
            is_focused: false,
            mode: InputMode::Compose,
            placeholder: "Type a message... (i to focus, Enter to send)",
            last_typing_sent: None,
        }
    }

    pub fn content(&self) -> String { self.textarea.lines().join("\n") }

    pub fn content_len(&self) -> usize {
        self.textarea.lines().iter().map(String::len).sum::<usize>()
            + self.textarea.lines().len().saturating_sub(1)
    }

    pub fn clear(&mut self) {
        self.textarea.select_all();
        self.textarea.cut();
        self.mode = InputMode::Compose;
    }

    pub fn set_mode(&mut self, mode: InputMode) { self.mode = mode; }
    pub fn mode(&self) -> &InputMode { &self.mode }
    pub fn is_over_limit(&self) -> bool { self.content_len() > MAX_MESSAGE_LENGTH }

    /// Insert text at the current cursor position.
    /// Used by file reference insertion (Ctrl+P → select file → insert path).
    pub fn insert_text(&mut self, text: &str) {
        self.textarea.insert_str(text);
    }

    /// Check whether a typing indicator should be emitted (rate-limited to 3s).
    /// Returns true if cooldown has elapsed or never sent. Resets the timer.
    pub fn should_emit_typing(&mut self) -> bool {
        let now = std::time::Instant::now();
        let should = self.last_typing_sent
            .is_none_or(|last| now.duration_since(last) >= TYPING_COOLDOWN);
        if should {
            self.last_typing_sent = Some(now);
        }
        should
    }

    fn title(&self) -> String {
        match &self.mode {
            InputMode::Compose => {
                let len = self.content_len();
                if len > 0 { format!(" Compose ({len}/{MAX_MESSAGE_LENGTH}) ") }
                else { " Compose ".into() }
            }
            InputMode::Reply { author, .. } => format!(" Reply to {author} "),
            InputMode::Edit { .. } => " Edit message ".into(),
        }
    }
}

impl Component for InputBox {
    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if !self.is_focused { return None; }

        match key.code {
            KeyCode::Esc => {
                if self.mode != InputMode::Compose {
                    self.mode = InputMode::Compose;
                    return None;
                }
                Some(Action::ExitInputMode)
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                let text = self.content();
                if text.trim().is_empty() { return None; }
                if self.is_over_limit() {
                    return Some(Action::ShowToast {
                        message: format!("Message too long ({} chars, max {MAX_MESSAGE_LENGTH})", self.content_len()),
                        level: ToastLevel::Warning,
                    });
                }
                Some(Action::InputSubmit)
            }
            KeyCode::Enter => {
                self.textarea.input(key);
                None
            }
            KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.textarea.undo();
                None
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.textarea.redo();
                None
            }
            _ => {
                if matches!(key.code, KeyCode::Char(_))
                    && self.content_len() >= MAX_MESSAGE_LENGTH
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    return None;
                }
                self.textarea.input(key);
                None
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.is_focused { Style::new() } else { Style::new().dim() };
        let block = Block::bordered().title(self.title()).border_style(border_style);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.textarea.lines().iter().all(String::is_empty) && !self.is_focused {
            frame.render_widget(
                Paragraph::new(Span::styled(self.placeholder, Style::new().dim().italic())),
                inner,
            );
        } else {
            frame.render_widget(&self.textarea, inner);
        }

        if self.is_over_limit() {
            let warning = Paragraph::new(format!(
                " {}/{MAX_MESSAGE_LENGTH} — too long! ", self.content_len()
            )).style(Style::new().bold());
            let warning_area = Rect {
                x: area.x + 1,
                y: area.bottom().saturating_sub(1),
                width: area.width.saturating_sub(2),
                height: 1,
            };
            frame.render_widget(warning, warning_area);
        }

    }

    fn set_focused(&mut self, focused: bool) { self.is_focused = focused; }
}
