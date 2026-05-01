//! Output mode detection and formatting dispatch.
//!
//! `OutputMode` is the single decision point for how the application
//! produces output. It is determined once at startup in `main.rs` and
//! passed to every command handler.

pub mod color;
pub mod format;
pub mod table;

/// Output mode — determines formatting, color, and TUI behavior.
///
/// Resolved once at startup via `detect()`. Never changes during a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Full TUI: alternate screen, raw mode, event loop.
    Tui,
    /// One-shot text: human-readable, color if TTY.
    Text,
    /// Structured JSON: machine-parseable, no color.
    Json,
    /// Structured JSONL: one object per line, for streaming pipelines.
    Jsonl,
}

impl OutputMode {
    /// Single source of truth for mode detection.
    ///
    /// Called once in `main.rs`. The result is passed everywhere.
    /// Priority: --format flag > pipe detection > TUI command detection.
    pub fn detect(format_flag: Option<&str>, is_tui_command: bool, no_color_flag: bool) -> Self {
        // --format always wins
        match format_flag {
            Some("json") => return Self::Json,
            Some("jsonl") => return Self::Jsonl,
            Some("text") => return Self::Text,
            _ => {}
        }

        // Piped stdout — never TUI, never color
        if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            return Self::Text;
        }

        // Explicit no-color still allows TUI (TUI respects theme tokens)
        let _ = no_color_flag;

        // TUI commands in a TTY
        if is_tui_command {
            return Self::Tui;
        }

        Self::Text
    }

    /// Whether this mode should use ANSI color in output.
    ///
    /// Delegates to `ColorSupport::detect` for the full detection chain
    /// (NO_COLOR env, TERM=dumb, TTY check, --no-color flag).
    pub fn use_color(self) -> bool {
        match self {
            Self::Json | Self::Jsonl => false,
            Self::Text | Self::Tui => {
                let support = color::ColorSupport::detect(false);
                support.is_enabled()
            }
        }
    }

    /// Detect the full color capability profile for the current terminal.
    ///
    /// Returns a `ColorSupport` with `is_enabled()`, `has_true_color()`,
    /// and `has_256_colors()` for use by the TUI theme layer and table
    /// formatting module.
    pub fn color_support(self) -> color::ColorSupport {
        match self {
            Self::Json | Self::Jsonl => color::ColorSupport::detect(true),
            Self::Text | Self::Tui => color::ColorSupport::detect(false),
        }
    }

    /// Whether this mode is a structured output format (JSON/JSONL).
    pub fn is_structured(self) -> bool {
        matches!(self, Self::Json | Self::Jsonl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_json_from_flag() {
        assert_eq!(OutputMode::detect(Some("json"), false, false), OutputMode::Json);
    }

    #[test]
    fn detect_jsonl_from_flag() {
        assert_eq!(OutputMode::detect(Some("jsonl"), false, false), OutputMode::Jsonl);
    }

    #[test]
    fn detect_text_from_flag() {
        assert_eq!(OutputMode::detect(Some("text"), true, false), OutputMode::Text);
    }

    #[test]
    fn detect_text_default() {
        // In test environment, stdout may or may not be a TTY
        let mode = OutputMode::detect(None, false, false);
        // Should be Text (non-TTY in test) or Text (TTY but not TUI command)
        assert!(matches!(mode, OutputMode::Text));
    }

    #[test]
    fn json_never_uses_color() {
        assert!(!OutputMode::Json.use_color());
        assert!(!OutputMode::Jsonl.use_color());
    }

    #[test]
    fn structured_check() {
        assert!(OutputMode::Json.is_structured());
        assert!(OutputMode::Jsonl.is_structured());
        assert!(!OutputMode::Text.is_structured());
        assert!(!OutputMode::Tui.is_structured());
    }
}
