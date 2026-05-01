//! Terminal color capability detection.
//!
//! Detects NO_COLOR, TERM, COLORTERM, and TTY status to determine
//! what level of color support the terminal provides. Used by the
//! output formatting layer to decide between RGB, 256-color, 16-color,
//! or no-color output.

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
    /// Detection order (first match wins):
    /// 1. `no_color_flag` (--no-color CLI flag) → disable all
    /// 2. `NO_COLOR` env variable → disable all
    /// 3. stdout is not a terminal → disable all
    /// 4. `TERM=dumb` → disable all
    /// 5. `COLORTERM=truecolor|24bit` → full RGB
    /// 6. `TERM` contains "256color" → 256-color palette
    /// 7. Default → basic 16-color
    pub fn detect(no_color_flag: bool) -> Self {
        if no_color_flag {
            return Self::none();
        }
        if std::env::var("NO_COLOR").is_ok() {
            return Self::none();
        }
        if !std::io::stdout().is_terminal() {
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

        Self {
            enabled: true,
            true_color,
            palette_256,
        }
    }

    /// No color support at all.
    fn none() -> Self {
        Self {
            enabled: false,
            true_color: false,
            palette_256: false,
        }
    }

    /// Whether any color output is supported.
    pub fn is_enabled(self) -> bool {
        self.enabled
    }

    /// Whether 24-bit RGB color is supported.
    ///
    /// Used by the TUI theme layer to select between RGB color tokens
    /// and 256-color approximations for the opaline theme palette.
    pub fn has_true_color(self) -> bool {
        self.true_color
    }

    /// Whether 256-color indexed palette is supported.
    ///
    /// Used by the output table module to decide whether to apply
    /// colored header styling (256-color terminals) or plain text.
    pub fn has_256_colors(self) -> bool {
        self.palette_256
    }

    /// Whether Unicode glyphs can be used (vs ASCII-only fallback).
    ///
    /// Heuristic: if TERM=dumb or LANG doesn't contain UTF, fall back to ASCII.
    pub fn use_unicode() -> bool {
        let term = std::env::var("TERM").unwrap_or_default();
        if term == "dumb" {
            return false;
        }
        // Check LANG for UTF-8 indicator
        let lang = std::env::var("LANG").unwrap_or_default();
        lang.contains("UTF") || lang.contains("utf")
            // Most modern terminals support Unicode even without LANG
            || std::io::stdout().is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_flag_disables() {
        let support = ColorSupport::detect(true);
        assert!(!support.is_enabled());
        assert!(!support.has_true_color());
        assert!(!support.has_256_colors());
    }
}
