//! Output mode detection and formatting dispatch.
//!
//! `OutputMode` is the single decision point for how the application
//! produces output. It is determined once at startup in `main.rs` and
//! passed to every command handler.

pub mod color;
pub mod format;
pub mod table;

/// Output mode — determines formatting and color.
///
/// Resolved once at startup via `detect()`. Never changes during a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable text, colored when the terminal supports it.
    Text,
    /// Structured JSON: machine-parseable, no color.
    Json,
    /// Structured JSONL: one object per line, for streaming pipelines.
    Jsonl,
}

impl OutputMode {
    /// Single source of truth for mode detection: `--format` wins, then
    /// `--script` (JSONL for streaming consumers), else text.
    pub fn detect(format_flag: Option<&str>, script_flag: bool) -> Self {
        match format_flag {
            Some("json") => Self::Json,
            Some("jsonl") => Self::Jsonl,
            Some("text") => Self::Text,
            _ if script_flag => Self::Jsonl,
            _ => Self::Text,
        }
    }

    /// Whether this mode should use ANSI color (`ColorSupport::detect`:
    /// `--no-color`, `NO_COLOR`, `TERM=dumb`, TTY).
    pub fn use_color(self) -> bool {
        self.color_support().is_enabled()
    }

    /// The color capability profile for this run.
    pub fn color_support(self) -> color::ColorSupport {
        match self {
            Self::Json | Self::Jsonl => color::ColorSupport::detect(true),
            Self::Text => color::ColorSupport::detect(color::no_color_flag()),
        }
    }

    /// Whether this mode is a structured output format (JSON/JSONL).
    pub fn is_structured(self) -> bool {
        matches!(self, Self::Json | Self::Jsonl)
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
