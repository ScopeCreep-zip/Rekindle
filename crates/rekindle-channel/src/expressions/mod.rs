//! Phase 19.g — pure expression validators + media-type detection.
//!
//! Ported from src-tauri/services/community/expressions.rs. Chiral
//! split: pure size / name / tag / magic-byte validators live here;
//! src-tauri retains the upload orchestrator (file write, DB persist,
//! gossip broadcast).

mod image;
mod limits;
mod ops;
mod upload;

pub use image::{
    prepare_upload, published_media_type, ExpressionPicture, PreparedPicture, EMOJI_MAX_SIDE,
    MAX_EXPRESSION_FRAMES, STICKER_MAX_SIDE,
};
pub use limits::{
    detect_audio_kind, normalize_tags, validate_expression_name, validate_soundboard_bytes,
    MAX_ANIMATED_EMOJI_BYTES, MAX_ANIMATED_EMOJI_COUNT, MAX_SOUNDBOARD_BYTES, MAX_SOUNDBOARD_COUNT,
    MAX_STATIC_EMOJI_BYTES, MAX_STATIC_EMOJI_COUNT, MAX_STICKER_BYTES, MAX_STICKER_COUNT,
};
pub use ops::{delete_expression, list_expressions, play_soundboard};
pub use upload::{
    upload_emoji, upload_soundboard_sound, upload_sticker, UploadSoundboardSoundParams,
};
