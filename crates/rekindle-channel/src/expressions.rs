//! Phase 19.g — pure expression validators + media-type detection.
//!
//! Ported from src-tauri/services/community/expressions.rs. Chiral
//! split: pure size / name / tag / magic-byte validators live here;
//! src-tauri retains the upload orchestrator (file write, DB persist,
//! gossip broadcast).

use crate::deps::{ChannelMessagingDeps, ExpressionView};
use crate::error::ChannelError;

/// Architecture §18.1: static emoji upload cap (256 KB).
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

use base64::Engine as _;
use rekindle_governance::state::ExpressionState;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::expression::SoundboardMeta;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

fn random_16_bytes() -> [u8; 16] {
    rand::random()
}

fn my_pseudonym_for_community<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
) -> Option<PseudonymKey> {
    let hex_str = deps.my_pseudonym_hex(community_id)?;
    let bytes = hex::decode(hex_str).ok()?;
    let arr: [u8; 32] = bytes.as_slice().try_into().ok()?;
    Some(PseudonymKey(arr))
}

fn enforce_count_limit<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    kind: &str,
    animated: bool,
    max: usize,
    label: &str,
) -> Result<(), ChannelError> {
    let gov = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    let count = gov
        .expressions
        .values()
        .filter(|expr| expr.kind == kind && expr.animated == animated)
        .count();
    if count >= max {
        Err(ChannelError::InvalidId(format!(
            "{label} limit reached ({count}/{max})"
        )))
    } else {
        Ok(())
    }
}

fn next_lamport<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
) -> Result<u64, ChannelError> {
    if deps.governance_state(community_id).is_none() {
        return Err(ChannelError::Adapter(
            "governance state not loaded for this community".into(),
        ));
    }
    Ok(deps.increment_lamport(community_id))
}

/// Phase 19.f — upload a custom emoji.
pub async fn upload_emoji<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    animated: bool,
) -> Result<String, ChannelError> {
    validate_expression_name(name)?;
    validate_emoji_bytes(&bytes, animated)?;
    let (max, kind_label) = if animated {
        (MAX_ANIMATED_EMOJI_COUNT, "animated emoji")
    } else {
        (MAX_STATIC_EMOJI_COUNT, "static emoji")
    };
    enforce_count_limit(deps, community_id, "emoji", animated, max, kind_label)?;

    let expression_id = random_16_bytes();
    let content_hash = blake3::hash(&bytes).to_hex().to_string();
    let creator = my_pseudonym_for_community(deps, community_id);
    let mime_type = if animated { "image/gif" } else { "image/png" }.to_string();
    let filename = format!("{name}.{}", if animated { "gif" } else { "png" });
    let attachment =
        deps.upload_expression_to_cache(community_id, expression_id, &bytes, filename, mime_type)?;

    let lamport = next_lamport(deps, community_id)?;
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name: name.to_string(),
            kind: "emoji".to_string(),
            content_hash,
            attachment: Some(attachment),
            animated,
            tags: Vec::new(),
            sound_meta: None,
            creator_pseudonym: creator,
            created_at: Some(rekindle_utils::timestamp_secs()),
            available_to_peers: Some(true),
            lamport,
        },
    )
    .await?;

    Ok(hex::encode(expression_id))
}

/// Phase 19.f — upload a custom sticker.
pub async fn upload_sticker<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    animated: bool,
    tags: Vec<String>,
) -> Result<String, ChannelError> {
    validate_expression_name(name)?;
    validate_sticker_bytes(&bytes, animated)?;
    enforce_count_limit(
        deps,
        community_id,
        "sticker",
        false,
        MAX_STICKER_COUNT,
        "sticker",
    )?;
    let normalized_tags = normalize_tags(tags)?;

    let expression_id = random_16_bytes();
    let content_hash = blake3::hash(&bytes).to_hex().to_string();
    let creator = my_pseudonym_for_community(deps, community_id);
    let mime_type = if animated { "image/apng" } else { "image/png" }.to_string();
    let filename = format!("{name}.{}", if animated { "apng" } else { "png" });
    let attachment =
        deps.upload_expression_to_cache(community_id, expression_id, &bytes, filename, mime_type)?;

    let lamport = next_lamport(deps, community_id)?;
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name: name.to_string(),
            kind: "sticker".to_string(),
            content_hash,
            attachment: Some(attachment),
            animated,
            tags: normalized_tags,
            sound_meta: None,
            creator_pseudonym: creator,
            created_at: Some(rekindle_utils::timestamp_secs()),
            available_to_peers: Some(true),
            lamport,
        },
    )
    .await?;

    Ok(hex::encode(expression_id))
}

/// Phase 19.f — upload a soundboard sound.
#[allow(
    clippy::too_many_arguments,
    reason = "Mirrors src-tauri upload_soundboard_sound signature; consolidating into a struct would just reshape the call site."
)]
pub async fn upload_soundboard_sound<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    tags: Vec<String>,
    duration_seconds: f32,
    volume: f32,
    emoji: Option<String>,
) -> Result<String, ChannelError> {
    validate_expression_name(name)?;
    validate_soundboard_bytes(&bytes)?;
    SoundboardMeta::validate_duration(duration_seconds)
        .map_err(|e| ChannelError::InvalidId(e.to_string()))?;
    SoundboardMeta::validate_volume(volume).map_err(|e| ChannelError::InvalidId(e.to_string()))?;
    SoundboardMeta::validate_emoji(emoji.as_deref())
        .map_err(|e| ChannelError::InvalidId(e.to_string()))?;
    enforce_count_limit(
        deps,
        community_id,
        "soundboard",
        false,
        MAX_SOUNDBOARD_COUNT,
        "soundboard sound",
    )?;
    let normalized_tags = normalize_tags(tags)?;

    let expression_id = random_16_bytes();
    let content_hash = blake3::hash(&bytes).to_hex().to_string();
    let sound_meta = SoundboardMeta {
        duration_seconds,
        volume,
        emoji,
    };
    let creator = my_pseudonym_for_community(deps, community_id);
    let attachment = deps.upload_expression_to_cache(
        community_id,
        expression_id,
        &bytes,
        format!("{name}.ogg"),
        "audio/ogg".to_string(),
    )?;

    let lamport = next_lamport(deps, community_id)?;
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name: name.to_string(),
            kind: "soundboard".to_string(),
            content_hash,
            attachment: Some(attachment),
            animated: false,
            tags: normalized_tags,
            sound_meta: Some(sound_meta),
            creator_pseudonym: creator,
            created_at: Some(rekindle_utils::timestamp_secs()),
            available_to_peers: Some(true),
            lamport,
        },
    )
    .await?;

    Ok(hex::encode(expression_id))
}

/// Phase 19.f — trigger a soundboard sound in a voice channel.
pub fn play_soundboard<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    expression_id_hex: &str,
) -> Result<(), ChannelError> {
    let expr = list_expressions(deps, community_id)?
        .into_iter()
        .find(|e| e.expression_id.eq_ignore_ascii_case(expression_id_hex))
        .ok_or_else(|| ChannelError::InvalidId("expression not found".into()))?;
    if expr.kind != "soundboard" {
        return Err(ChannelError::InvalidId(
            "expression is not a soundboard sound".into(),
        ));
    }
    let actor_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    let envelope = CommunityEnvelope::Control(ControlPayload::SoundboardPlay {
        channel_id: channel_id.to_string(),
        expression_id: expression_id_hex.to_string(),
        actor_pseudonym,
    });
    deps.send_to_mesh(community_id, &envelope)
}

/// Phase 19.f — delete a custom expression.
pub async fn delete_expression<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    expression_id_hex: &str,
) -> Result<(), ChannelError> {
    let expression_id: [u8; 16] = hex::decode(expression_id_hex)
        .map_err(|e| ChannelError::InvalidId(format!("invalid expression id: {e}")))?
        .try_into()
        .map_err(|_| ChannelError::InvalidId("expression id must be 16 bytes".into()))?;

    let lamport = next_lamport(deps, community_id)?;
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport,
        },
    )
    .await
}

/// Phase 19.f — list all expressions in a community, sorted by name.
pub fn list_expressions<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
) -> Result<Vec<ExpressionView>, ChannelError> {
    let gov = deps.governance_state(community_id).ok_or_else(|| {
        ChannelError::Adapter("governance state not loaded for this community".into())
    })?;
    let mut expressions: Vec<_> = gov
        .expressions
        .into_iter()
        .map(|(expression_id, expression)| {
            to_expression_view(deps, community_id, expression_id, expression)
        })
        .collect();
    expressions.sort_by(|l, r| {
        l.name
            .cmp(&r.name)
            .then_with(|| l.expression_id.cmp(&r.expression_id))
    });
    Ok(expressions)
}

fn to_expression_view<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    expression_id: [u8; 16],
    expression: ExpressionState,
) -> ExpressionView {
    let bytes = expression
        .attachment
        .as_ref()
        .and_then(|offer| deps.read_expression_bytes(community_id, offer));
    let media_type = bytes
        .as_deref()
        .and_then(|b| detect_image_media_type(b, expression.animated))
        .map(str::to_string);
    let inline_data_base64 = bytes
        .as_deref()
        .map(|b| base64::engine::general_purpose::STANDARD.encode(b));

    ExpressionView {
        expression_id: hex::encode(expression_id),
        name: expression.name,
        kind: expression.kind,
        content_hash: expression.content_hash,
        inline_data_base64,
        media_type,
        animated: expression.animated,
        tags: expression.tags,
        sound_meta: expression.sound_meta,
        creator_pseudonym: expression.creator_pseudonym.map(|p| hex::encode(p.0)),
        created_at: expression.created_at,
        available_to_peers: expression.available_to_peers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_length_bounds() {
        assert!(validate_expression_name("ab").is_ok());
        assert!(validate_expression_name(&"x".repeat(32)).is_ok());
        assert!(validate_expression_name("a").is_err());
        assert!(validate_expression_name(&"x".repeat(33)).is_err());
    }

    #[test]
    fn name_validation_charset() {
        assert!(validate_expression_name("abc_123").is_ok());
        assert!(validate_expression_name("hello-world").is_err());
        assert!(validate_expression_name("hello world").is_err());
        assert!(validate_expression_name("héllo").is_err());
    }

    #[test]
    fn normalize_tags_trims_lowercases_dedupes() {
        let out = normalize_tags(vec![
            "  Foo ".into(),
            "FOO".into(),
            "bar-baz".into(),
            String::new(),
        ])
        .unwrap();
        assert_eq!(out, vec!["foo", "bar-baz"]);
    }

    #[test]
    fn normalize_tags_rejects_overlimits() {
        assert!(normalize_tags(vec!["x".into(); 17]).is_err());
        assert!(normalize_tags(vec!["x".repeat(25)]).is_err());
        assert!(normalize_tags(vec!["bad!tag".into()]).is_err());
    }

    #[test]
    fn detect_png_magic() {
        let png = b"\x89PNG\r\n\x1a\n....";
        assert_eq!(detect_image_media_type(png, false), Some("image/png"));
    }

    #[test]
    fn detect_webp_magic() {
        let webp = b"RIFF....WEBPmore";
        assert_eq!(detect_image_media_type(webp, false), Some("image/webp"));
    }

    #[test]
    fn detect_gif_only_when_animated_allowed() {
        let gif = b"GIF89a....";
        assert_eq!(detect_image_media_type(gif, true), Some("image/gif"));
        assert_eq!(detect_image_media_type(gif, false), None);
    }

    #[test]
    fn detect_audio_ogg_webm_mp3() {
        assert_eq!(detect_audio_kind(b"OggS...."), Some("audio/ogg"));
        assert_eq!(
            detect_audio_kind(b"\x1A\x45\xDF\xA3...."),
            Some("audio/webm")
        );
        assert_eq!(detect_audio_kind(b"ID3...."), Some("audio/mpeg"));
        // MP3 sync word
        assert_eq!(
            detect_audio_kind(&[0xFF, 0xFB, 0x90, 0x00]),
            Some("audio/mpeg")
        );
        assert_eq!(detect_audio_kind(b"random"), None);
    }

    #[test]
    fn validate_emoji_rejects_empty_and_oversize() {
        assert!(validate_emoji_bytes(&[], false).is_err());
        let oversize = vec![0u8; MAX_STATIC_EMOJI_BYTES + 1];
        assert!(matches!(
            validate_emoji_bytes(&oversize, false),
            Err(ChannelError::BodyTooLarge { .. })
        ));
    }

    #[test]
    fn validate_emoji_accepts_png() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0u8; 100]);
        assert!(validate_emoji_bytes(&png, false).is_ok());
    }

    #[test]
    fn validate_sticker_animated_rejects_static_gif_unless_allowed() {
        let gif = b"GIF89a....".to_vec();
        assert!(validate_sticker_bytes(&gif, true).is_ok());
        assert!(validate_sticker_bytes(&gif, false).is_err());
    }

    #[test]
    fn validate_soundboard_recognises_ogg() {
        let mut ogg = b"OggS".to_vec();
        ogg.extend_from_slice(&[0u8; 100]);
        assert!(validate_soundboard_bytes(&ogg).is_ok());
        assert!(validate_soundboard_bytes(b"raw bytes").is_err());
    }
}
