//! Terminal color capability detection.
//!
//! Detects NO_COLOR, TERM, COLORTERM, and TTY status to determine
//! color support level. Used by output formatting and TUI theme.

use std::io::IsTerminal;

/// Detected color support level for the current terminal.
#[derive(Debug, Clone, Copy)]
pub struct ColorSupport {
    /// Whether any color is enabled.
    pub enabled: bool,
    /// Whether 24-bit true color (RGB) is supported.
    pub true_color: bool,
    /// Whether 256-color palette is supported.
    pub palette_256: bool,
}

impl ColorSupport {
    /// Detect color support from environment and terminal state.
    ///
    /// Detection order:
    /// 1. `no_color_flag` (--no-color CLI flag) → disable all
    /// 2. `NO_COLOR` env variable → disable all
    /// 3. stdout is not a terminal → disable all
    /// 4. `TERM=dumb` → disable all
    /// 5. `COLORTERM=truecolor|24bit` → full RGB
    /// 6. `TERM` contains "256color" → 256-color palette
    /// 7. Default → basic 16-color
    pub fn detect(no_color_flag: bool) -> Self {
        if no_color_flag || std::env::var("NO_COLOR").is_ok() || !std::io::stdout().is_terminal() {
            return Self::none();
        }
        let term = std::env::var("TERM").unwrap_or_default();
        if term == "dumb" {
            return Self::none();
        }

        let true_color = std::env::var("COLORTERM")
            .map(|v| v == "truecolor" || v == "24bit")
            .unwrap_or(false);

        let palette_256 = term.contains("256color") || true_color;

        Self { enabled: true, true_color, palette_256 }
    }

    fn none() -> Self {
        Self { enabled: false, true_color: false, palette_256: false }
    }

    pub fn is_enabled(self) -> bool { self.enabled }
    pub fn has_true_color(self) -> bool { self.true_color }
    pub fn has_256_colors(self) -> bool { self.palette_256 }

    /// Whether Unicode glyphs can be used (vs ASCII-only fallback).
    pub fn use_unicode() -> bool {
        let term = std::env::var("TERM").unwrap_or_default();
        if term == "dumb" {
            return false;
        }
        let lang = std::env::var("LANG").unwrap_or_default();
        lang.contains("UTF") || lang.contains("utf") || std::io::stdout().is_terminal()
    }
}
