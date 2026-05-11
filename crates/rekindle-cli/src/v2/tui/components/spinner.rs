//! Loading spinner — braille/ASCII rotating animation.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const BRAILLE_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_FRAMES: &[&str] = &["|", "/", "-", "\\"];

/// Loading spinner with optional label.
pub struct Spinner {
    frame: usize,
    animated: bool,
    unicode: bool,
    label: String,
    active: bool,
}

impl Spinner {
    pub fn new(animated: bool, unicode: bool) -> Self {
        Self { frame: 0, animated, unicode, label: String::new(), active: false }
    }

    pub fn set_label(&mut self, label: impl Into<String>) { self.label = label.into(); }
    pub fn start(&mut self) { self.active = true; self.frame = 0; }
    pub fn stop(&mut self) { self.active = false; }
    pub fn is_active(&self) -> bool { self.active }

    pub fn tick(&mut self) {
        if self.active { self.frame = self.frame.wrapping_add(1); }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.active { return; }

        let glyph = if self.animated {
            let frames = if self.unicode { BRAILLE_FRAMES } else { ASCII_FRAMES };
            frames[self.frame % frames.len()]
        } else {
            "[...]"
        };

        let text = if self.label.is_empty() {
            glyph.to_string()
        } else {
            format!("{glyph} {}", self.label)
        };

        frame.render_widget(Paragraph::new(Span::styled(text, Style::new().dim())), area);
    }
}
