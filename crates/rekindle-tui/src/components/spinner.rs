//! Loading spinner component.
//!
//! Renders a rotating character animation for async operations in
//! progress. Stateless — animation frame computed from a frame counter.
//! Disabled when `config.tui.animations` is false, in which case it
//! renders a static `[...]` indicator.
//!
//! Braille spinner frames give smooth motion in Unicode terminals.
//! Falls back to ASCII `|/-\` when `use_unicode` is false.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Unicode braille spinner frames — 10 frames for smooth rotation.
const BRAILLE_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// ASCII fallback spinner frames.
const ASCII_FRAMES: &[&str] = &["|", "/", "-", "\\"];

/// Loading spinner — renders a single-character rotating animation.
pub struct Spinner {
    /// Frame counter — incremented each Tick.
    frame: usize,
    /// Whether animation is enabled.
    animated: bool,
    /// Whether Unicode glyphs are available.
    unicode: bool,
    /// Optional label displayed after the spinner.
    label: String,
    /// Whether the spinner is actively spinning.
    active: bool,
}

impl Spinner {
    /// Create a new spinner.
    pub fn new(animated: bool, unicode: bool) -> Self {
        Self {
            frame: 0,
            animated,
            unicode,
            label: String::new(),
            active: false,
        }
    }

    /// Set the label displayed next to the spinner.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// Start spinning.
    pub fn start(&mut self) {
        self.active = true;
        self.frame = 0;
    }

    /// Stop spinning.
    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Whether the spinner is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Advance the animation by one frame. Called on each Tick.
    pub fn tick(&mut self) {
        if self.active {
            self.frame = self.frame.wrapping_add(1);
        }
    }

    /// Render the spinner into the given area (single line).
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.active {
            return;
        }

        let glyph = if self.animated {
            let frames = if self.unicode {
                BRAILLE_FRAMES
            } else {
                ASCII_FRAMES
            };
            frames[self.frame % frames.len()]
        } else {
            // Static indicator when animations disabled
            "[...]"
        };

        let text = if self.label.is_empty() {
            glyph.to_string()
        } else {
            format!("{glyph} {}", self.label)
        };

        let para = Paragraph::new(Span::styled(text, Style::new().dim()));
        frame.render_widget(para, area);
    }
}

#[cfg(test)]
#[path = "spinner_tests.rs"]
mod tests;
