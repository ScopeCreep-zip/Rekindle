//! Architecture §18 — expression-asset metadata (emoji, stickers, soundboard).
//!
//! `ExpressionAdded` governance entries already carry the common fields
//! (id, name, kind, content_hash, inline_data, animated, tags). Soundboard
//! sounds need three extra fields per §18.3 line 2491-2493 — kept on a
//! distinct struct so emoji/sticker entries pay no wire cost.

use serde::{Deserialize, Serialize};

/// Soundboard-specific metadata. Architecture §18.3:
/// - `duration_seconds` is bounded to `(0.0, 5.0]` (line 2491).
/// - `volume` is bounded to `[0.0, 1.0]` (line 2492). Receivers multiply
///   their per-channel volume by this so uploaders can normalise loud
///   clips without forcing every listener to ride the slider.
/// - `emoji` is an optional Unicode glyph the picker shows next to the
///   sound name (line 2493).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundboardMeta {
    pub duration_seconds: f32,
    pub volume: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

impl SoundboardMeta {
    /// Architecture §18.3 line 2491 — clips longer than five seconds are
    /// rejected. Zero / negative durations are nonsense and also rejected.
    pub const MAX_DURATION_SECS: f32 = 5.0;

    /// Reject NaN, ≤0, or >5 seconds. Returns the reason on failure.
    pub fn validate_duration(duration_seconds: f32) -> Result<(), &'static str> {
        if !duration_seconds.is_finite() {
            return Err("soundboard duration must be a finite number");
        }
        if duration_seconds <= 0.0 {
            return Err("soundboard duration must be positive");
        }
        if duration_seconds > Self::MAX_DURATION_SECS {
            return Err("soundboard duration exceeds 5.0 seconds (architecture §18.3)");
        }
        Ok(())
    }

    /// Reject NaN or values outside `[0.0, 1.0]`.
    pub fn validate_volume(volume: f32) -> Result<(), &'static str> {
        if !volume.is_finite() {
            return Err("soundboard volume must be a finite number");
        }
        if !(0.0..=1.0).contains(&volume) {
            return Err("soundboard volume must be in 0.0..=1.0");
        }
        Ok(())
    }

    /// Reject overlong glyph strings — emoji is a *single* glyph in the UI,
    /// not a label. 16 chars allows for ZWJ-joined sequences (👨‍👩‍👧‍👦).
    pub fn validate_emoji(emoji: Option<&str>) -> Result<(), &'static str> {
        match emoji {
            Some("") => Err("soundboard emoji must not be empty when set"),
            Some(s) if s.chars().count() > 16 => {
                Err("soundboard emoji must be ≤16 chars (a single glyph or ZWJ sequence)")
            }
            _ => Ok(()),
        }
    }
}
