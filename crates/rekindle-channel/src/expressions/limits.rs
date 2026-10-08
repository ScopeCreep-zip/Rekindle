//! Size/count limits and byte-format validators for expressions.

use crate::error::ChannelError;

pub const MAX_STATIC_EMOJI_BYTES: usize = 256 * 1024;
/// Architecture §18.1: animated emoji upload cap (512 KB).
pub const MAX_ANIMATED_EMOJI_BYTES: usize = 512 * 1024;
/// Architecture §18.2: sticker upload cap (1 MB).
pub const MAX_STICKER_BYTES: usize = 1024 * 1024;
/// Architecture §18.3: soundboard clip cap (1 MB).
pub const MAX_SOUNDBOARD_BYTES: usize = 1024 * 1024;

/// Per-community limits (architecture §18.1/2/3).
pub const MAX_STATIC_EMOJI_COUNT: usize = 50;
pub const MAX_ANIMATED_EMOJI_COUNT: usize = 50;
pub const MAX_STICKER_COUNT: usize = 30;
pub const MAX_SOUNDBOARD_COUNT: usize = 48;

/// Architecture §18 — shared `:name:` validator for emoji, stickers,
/// and soundboard clips. 2-32 ASCII alphanumeric + underscore only.
pub fn validate_expression_name(name: &str) -> Result<(), ChannelError> {
    if !(2..=32).contains(&name.len()) {
        return Err(ChannelError::InvalidId(
            "expression name must be 2-32 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(ChannelError::InvalidId(
            "expression name may only contain letters, numbers, and underscores".into(),
        ));
    }
    Ok(())
}

/// Normalise + dedupe + validate the discovery tags attached to an
/// expression. Returns the cleaned (lowercase + trimmed, duplicates
/// removed) tag list.
///
/// Constraints (architecture §18): ≤16 tags, each ≤24 chars, ASCII
/// alphanumeric + `-` or `_`.
pub fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, ChannelError> {
    if tags.len() > 16 {
        return Err(ChannelError::InvalidId(
            "expression supports at most 16 tags".into(),
        ));
    }
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let trimmed = tag.trim().to_lowercase();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.len() > 24 {
            return Err(ChannelError::InvalidId(
                "expression tag must be ≤24 characters".into(),
            ));
        }
        if !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return Err(ChannelError::InvalidId(
                "expression tag may only contain letters, numbers, '-' or '_'".into(),
            ));
        }
        if !out.contains(&trimmed) {
            out.push(trimmed);
        }
    }
    Ok(out)
}

/// Sniff an image-upload payload by magic bytes. Returns the mime
/// content-type if recognised, `None` otherwise. Pure — does NOT
/// decode the image, just verifies the container claim is plausible.
///
/// Supports: PNG, WebP, GIF (animated only).
#[must_use]
pub fn detect_image_media_type(bytes: &[u8], animated_allowed: bool) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if animated_allowed && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some("image/gif");
    }
    None
}

/// Sniff a soundboard payload's container. Architecture §18.3 only
/// allows small Opus or MP3 clips.
#[must_use]
pub fn detect_audio_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"OggS") {
        return Some("audio/ogg");
    }
    if bytes.len() >= 4 && bytes.starts_with(b"\x1A\x45\xDF\xA3") {
        return Some("audio/webm");
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
    {
        return Some("audio/mpeg");
    }
    None
}

/// Validate emoji bytes: non-empty, ≤ size cap (static or animated),
/// recognised image format.
pub fn validate_emoji_bytes(bytes: &[u8], animated: bool) -> Result<(), ChannelError> {
    let max_len = if animated {
        MAX_ANIMATED_EMOJI_BYTES
    } else {
        MAX_STATIC_EMOJI_BYTES
    };
    if bytes.is_empty() {
        return Err(ChannelError::InvalidId(
            "emoji upload cannot be empty".into(),
        ));
    }
    if bytes.len() > max_len {
        return Err(ChannelError::BodyTooLarge {
            size: bytes.len(),
            max: max_len,
        });
    }
    if detect_image_media_type(bytes, animated).is_none() {
        return Err(ChannelError::InvalidId(if animated {
            "animated emoji must be PNG, WebP, or GIF".into()
        } else {
            "emoji must be PNG or WebP".into()
        }));
    }
    Ok(())
}

/// Validate sticker bytes: non-empty, ≤ 1 MB, recognised image format.
pub fn validate_sticker_bytes(bytes: &[u8], animated: bool) -> Result<(), ChannelError> {
    if bytes.is_empty() {
        return Err(ChannelError::InvalidId(
            "sticker upload cannot be empty".into(),
        ));
    }
    if bytes.len() > MAX_STICKER_BYTES {
        return Err(ChannelError::BodyTooLarge {
            size: bytes.len(),
            max: MAX_STICKER_BYTES,
        });
    }
    if detect_image_media_type(bytes, animated).is_none() {
        return Err(ChannelError::InvalidId(if animated {
            "animated sticker must be PNG, WebP, or GIF".into()
        } else {
            "sticker must be PNG or WebP".into()
        }));
    }
    Ok(())
}

/// Validate soundboard bytes: non-empty, ≤ 1 MB, recognised audio container.
pub fn validate_soundboard_bytes(bytes: &[u8]) -> Result<(), ChannelError> {
    if bytes.is_empty() {
        return Err(ChannelError::InvalidId(
            "soundboard upload cannot be empty".into(),
        ));
    }
    if bytes.len() > MAX_SOUNDBOARD_BYTES {
        return Err(ChannelError::BodyTooLarge {
            size: bytes.len(),
            max: MAX_SOUNDBOARD_BYTES,
        });
    }
    if detect_audio_kind(bytes).is_none() {
        return Err(ChannelError::InvalidId(
            "soundboard sound must be Opus (OGG/WebM) or MP3".into(),
        ));
    }
    Ok(())
}

// ---------- 19.f-REDO: full expression pipeline ----------

#[cfg(test)]
mod tests;
