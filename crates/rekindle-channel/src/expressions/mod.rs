//! Phase 19.g — pure expression validators + media-type detection.
//!
//! Ported from src-tauri/services/community/expressions.rs. Chiral
//! split: pure size / name / tag / magic-byte validators live here;
//! src-tauri retains the upload orchestrator (file write, DB persist,
//! gossip broadcast).

mod limits;
mod ops;
mod upload;

pub use limits::{
    detect_audio_kind, detect_image_media_type, normalize_tags, validate_emoji_bytes,
    validate_expression_name, validate_soundboard_bytes, validate_sticker_bytes,
    MAX_ANIMATED_EMOJI_BYTES, MAX_ANIMATED_EMOJI_COUNT, MAX_SOUNDBOARD_BYTES, MAX_SOUNDBOARD_COUNT,
    MAX_STATIC_EMOJI_BYTES, MAX_STATIC_EMOJI_COUNT, MAX_STICKER_BYTES, MAX_STICKER_COUNT,
};
pub use ops::{delete_expression, list_expressions, play_soundboard};
pub use upload::{
    upload_emoji, upload_soundboard_sound, upload_sticker, UploadSoundboardSoundParams,
};
