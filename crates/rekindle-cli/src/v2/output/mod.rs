//! Output mode detection and formatting dispatch.
//!
//! `OutputMode` is the single decision point for how the application
//! produces output. Determined once at startup, passed everywhere.

pub mod color;
pub mod format;
pub mod table;

/// Output mode — determines formatting, color, and TUI behavior.
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
    /// Detect output mode from flags and environment.
    ///
    /// Priority: --format flag > --script > pipe detection > TUI command.
    pub fn detect(
        format_flag: Option<&str>,
        is_tui_command: bool,
        no_color_flag: bool,
        script_flag: bool,
    ) -> Self {
        match format_flag {
            Some("json") => return Self::Json,
            Some("jsonl") => return Self::Jsonl,
            Some("text") => return Self::Text,
            _ => {}
        }

        if script_flag {
            return Self::Jsonl;
        }

        if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            return Self::Text;
        }

        let _ = no_color_flag;

        if is_tui_command {
            return Self::Tui;
        }

        Self::Text
    }

    /// Whether ANSI color should be used in output.
    pub fn use_color(self) -> bool {
        match self {
            Self::Json | Self::Jsonl => false,
            Self::Text | Self::Tui => color::ColorSupport::detect(false).is_enabled(),
        }
    }

    /// Full color capability profile.
    pub fn color_support(self) -> color::ColorSupport {
        match self {
            Self::Json | Self::Jsonl => color::ColorSupport::detect(true),
            Self::Text | Self::Tui => color::ColorSupport::detect(false),
        }
    }

    /// Whether this is a structured output format (JSON/JSONL).
    pub fn is_structured(self) -> bool {
        matches!(self, Self::Json | Self::Jsonl)
    }
}
