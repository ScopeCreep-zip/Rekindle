use base64::Engine as _;
use rand::RngCore;
use rekindle_governance::state::ExpressionState;
use rekindle_types::expression::SoundboardMeta;
use rekindle_types::governance::GovernanceEntry;

use crate::state::SharedState;
use crate::state_helpers;

use super::governance;

const MAX_STATIC_EMOJI_BYTES: usize = 256 * 1024;
const MAX_ANIMATED_EMOJI_BYTES: usize = 512 * 1024;
/// Architecture §18.2: stickers larger than emoji (typically full-image
/// reactions). Cap at 1 MiB so the eager-cache budget (~48 MiB total
/// per community per §18.4) holds at ≥48 stickers.
const MAX_STICKER_BYTES: usize = 1024 * 1024;
/// Architecture §18.3: soundboard clips are short Opus/MP3, ≤1 MiB.
/// Lost Cargo handles anything bigger via chunked attachments, but
/// expression assets are inline for eager caching.
const MAX_SOUNDBOARD_BYTES: usize = 1024 * 1024;
/// Architecture §18.1 — "default 100: 50 static + 50 animated". The
/// CRDT validation in `rekindle_governance::validate` enforces these
/// post-merge, but we also pre-flight here so the user gets an
/// immediate "limit reached" error instead of a silent rejection by
/// peers after the upload round-trips.
const MAX_STATIC_EMOJI_COUNT: usize = 50;
const MAX_ANIMATED_EMOJI_COUNT: usize = 50;
/// Architecture §18.2 — "default 30".
const MAX_STICKER_COUNT: usize = 30;
/// Architecture §18.3 — "default 48".
const MAX_SOUNDBOARD_COUNT: usize = 48;

#[derive(Debug, Clone)]
pub struct ExpressionInfo {
    pub expression_id: String,
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    pub inline_data_base64: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
    /// Architecture §18.3 — present only on `kind == "soundboard"`.
    pub sound_meta: Option<SoundboardMeta>,
    /// Architecture §18.1 line 2455 — uploader's per-community pseudonym (hex).
    pub creator_pseudonym: Option<String>,
    /// Architecture §18.1 line 2456 — wall-clock seconds at upload.
    pub created_at: Option<u64>,
    /// Architecture §18.1 line 2459 — gates `USE_EXTERNAL_EMOJIS`.
    pub available_to_peers: bool,
}

pub async fn upload_emoji(
    state: &SharedState,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    animated: bool,
) -> Result<String, String> {
    validate_emoji_name(name)?;
    validate_emoji_bytes(&bytes, animated)?;
    let (max, kind_label) = if animated {
        (MAX_ANIMATED_EMOJI_COUNT, "animated emoji")
    } else {
        (MAX_STATIC_EMOJI_COUNT, "static emoji")
    };
    enforce_count_limit(state, community_id, "emoji", animated, max, kind_label)?;

    let expression_id = random_16_bytes();
    let content_hash = blake3::hash(&bytes).to_hex().to_string();
    let creator = my_pseudonym_for_community(state, community_id);
    let mime_type = if animated { "image/gif" } else { "image/png" }.to_string();
    let filename = format!("{name}.{}", if animated { "gif" } else { "png" });
    let attachment = super::expression_assets::upload_to_cache(
        state,
        community_id,
        expression_id,
        &bytes,
        filename,
        mime_type,
    )?;

    governance::write_entry(
        state,
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
            lamport: next_lamport(state, community_id)?,
        },
    )
    .await?;

    Ok(hex::encode(expression_id))
}

/// Architecture §18.1 line 2455 — resolve the uploader's per-community
/// pseudonym key into the wire form. Returns `None` only when there is
/// no governance state loaded (e.g. uploading before MEK rotation
/// finishes); the wire field is `Option<PseudonymKey>` precisely for
/// this back-compat case.
fn my_pseudonym_for_community(
    state: &SharedState,
    community_id: &str,
) -> Option<rekindle_types::id::PseudonymKey> {
    let communities = state.communities.read();
    let hex_str = communities
        .get(community_id)
        .and_then(|c| c.my_pseudonym_key.clone())?;
    let bytes = hex::decode(hex_str).ok()?;
    let arr: [u8; 32] = bytes.as_slice().try_into().ok()?;
    Some(rekindle_types::id::PseudonymKey(arr))
}

/// Architecture §18.2: stickers ship as inline-cached expression assets
/// alongside emoji. Same governance entry shape, larger size budget,
/// distinct `kind` so the frontend can render them in their own picker.
pub async fn upload_sticker(
    state: &SharedState,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    animated: bool,
    tags: Vec<String>,
) -> Result<String, String> {
    validate_expression_name(name)?;
    validate_sticker_bytes(&bytes, animated)?;
    enforce_count_limit(
        state,
        community_id,
        "sticker",
        false,
        MAX_STICKER_COUNT,
        "sticker",
    )?;
    let normalized_tags = normalize_tags(tags)?;

    let expression_id = random_16_bytes();
    let content_hash = blake3::hash(&bytes).to_hex().to_string();
    let creator = my_pseudonym_for_community(state, community_id);
    let mime_type = if animated { "image/apng" } else { "image/png" }.to_string();
    let filename = format!("{name}.{}", if animated { "apng" } else { "png" });
    let attachment = super::expression_assets::upload_to_cache(
        state,
        community_id,
        expression_id,
        &bytes,
        filename,
        mime_type,
    )?;

    governance::write_entry(
        state,
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
            lamport: next_lamport(state, community_id)?,
        },
    )
    .await?;

    Ok(hex::encode(expression_id))
}

/// Architecture §18.3: soundboard sounds are short Opus/MP3 clips
/// triggered by `SoundboardPlay` gossip (§10.9). Stored as expression
/// assets so they're eagerly cached on community join.
///
/// `duration_seconds` is reported by the uploader (Web Audio
/// `decodeAudioData` measures the decoded length; the OGG/MP3 container
/// itself doesn't always carry an authoritative duration). Validated to
/// `(0.0, 5.0]` here so peers reject obvious lies before merge.
/// `volume` is `0.0..=1.0` and is multiplied into each listener's
/// per-channel mix so an uploader can normalise loud clips. `emoji` is
/// the optional Unicode glyph the picker shows next to the sound name.
pub async fn upload_soundboard_sound(
    state: &SharedState,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    tags: Vec<String>,
    duration_seconds: f32,
    volume: f32,
    emoji: Option<String>,
) -> Result<String, String> {
    validate_expression_name(name)?;
    validate_soundboard_bytes(&bytes)?;
    SoundboardMeta::validate_duration(duration_seconds).map_err(str::to_string)?;
    SoundboardMeta::validate_volume(volume).map_err(str::to_string)?;
    SoundboardMeta::validate_emoji(emoji.as_deref()).map_err(str::to_string)?;
    enforce_count_limit(
        state,
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
    let creator = my_pseudonym_for_community(state, community_id);
    let attachment = super::expression_assets::upload_to_cache(
        state,
        community_id,
        expression_id,
        &bytes,
        format!("{name}.ogg"),
        "audio/ogg".to_string(),
    )?;

    governance::write_entry(
        state,
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
            lamport: next_lamport(state, community_id)?,
        },
    )
    .await?;

    Ok(hex::encode(expression_id))
}

/// Architecture §10.9: trigger a soundboard sound in a voice channel.
/// Verifies the expression exists and is `kind: "soundboard"`, then
/// broadcasts `SoundboardPlay` so every voice participant plays the
/// already-cached audio locally.
pub fn play_soundboard(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    expression_id_hex: &str,
) -> Result<(), String> {
    let expr = list_expressions(state, community_id)?
        .into_iter()
        .find(|e| e.expression_id.eq_ignore_ascii_case(expression_id_hex))
        .ok_or_else(|| "expression not found".to_string())?;
    if expr.kind != "soundboard" {
        return Err("expression is not a soundboard sound".into());
    }

    let actor_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
            .ok_or_else(|| "not a member of this community".to_string())?
    };

    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
    let envelope = CommunityEnvelope::Control(ControlPayload::SoundboardPlay {
        channel_id: channel_id.to_string(),
        expression_id: expression_id_hex.to_string(),
        actor_pseudonym,
    });
    super::send_to_mesh(state, community_id, &envelope)
}

pub async fn delete_expression(
    state: &SharedState,
    community_id: &str,
    expression_id_hex: &str,
) -> Result<(), String> {
    let expression_id: [u8; 16] = hex::decode(expression_id_hex)
        .map_err(|e| format!("invalid expression id: {e}"))?
        .try_into()
        .map_err(|_| "expression id must be 16 bytes")?;

    governance::write_entry(
        state,
        community_id,
        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport: next_lamport(state, community_id)?,
        },
    )
    .await
}

pub fn list_expressions(
    state: &SharedState,
    community_id: &str,
) -> Result<Vec<ExpressionInfo>, String> {
    let mut expressions: Vec<_> = state_helpers::governance_state(state, community_id)
        .ok_or("governance state not loaded for this community")?
        .expressions
        .into_iter()
        .map(|(expression_id, expression)| {
            to_expression_info(state, community_id, expression_id, expression)
        })
        .collect();

    expressions.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.expression_id.cmp(&right.expression_id))
    });

    Ok(expressions)
}

fn next_lamport(state: &SharedState, community_id: &str) -> Result<u64, String> {
    if state_helpers::governance_state(state, community_id).is_none() {
        return Err("governance state not loaded for this community".into());
    }
    Ok(state_helpers::increment_lamport(state, community_id))
}

/// Architecture §18.1/§18.2/§18.3 — pre-flight count check that
/// mirrors the post-merge gate in `rekindle_governance::validate`.
/// Lets the user see "you've hit the 50-emoji limit" instantly
/// instead of after a network round-trip.
fn enforce_count_limit(
    state: &SharedState,
    community_id: &str,
    kind: &str,
    animated: bool,
    max: usize,
    label: &str,
) -> Result<(), String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let Some(gov) = community.governance_state.as_ref() else {
        return Err("governance state not loaded".into());
    };
    let count = gov
        .expressions
        .values()
        .filter(|expr| expr.kind == kind && expr.animated == animated)
        .count();
    if count >= max {
        Err(format!("{label} limit reached ({count}/{max})"))
    } else {
        Ok(())
    }
}

fn to_expression_info(
    state: &SharedState,
    community_id: &str,
    expression_id: [u8; 16],
    expression: ExpressionState,
) -> ExpressionInfo {
    // Architecture §18.4: load the asset bytes from the local file cache
    // (via the wrapped FEK in the AttachmentOffer). Missing assets surface
    // as `inline_data_base64 = None`; the eager-fetch loop on the next
    // governance merge will pull them from a peer.
    let bytes = expression.attachment.as_ref().and_then(|offer| {
        super::expression_assets::read_bytes_from_cache(state, community_id, offer)
    });
    let media_type = bytes
        .as_deref()
        .and_then(|b| detect_media_type(b, expression.animated))
        .map(str::to_string);
    let inline_data_base64 = bytes
        .as_deref()
        .map(|b| base64::engine::general_purpose::STANDARD.encode(b));

    ExpressionInfo {
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

fn validate_emoji_name(name: &str) -> Result<(), String> {
    validate_expression_name(name)
}

/// Shared name validator for emoji, stickers, and soundboard sounds —
/// architecture §18.1/§18.2/§18.3 all use the same `:name:` shape.
fn validate_expression_name(name: &str) -> Result<(), String> {
    if !(2..=32).contains(&name.len()) {
        return Err("expression name must be 2-32 characters".into());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err("expression name may only contain letters, numbers, and underscores".into());
    }
    Ok(())
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, String> {
    if tags.len() > 16 {
        return Err("expression supports at most 16 tags".into());
    }
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let trimmed = tag.trim().to_lowercase();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.len() > 24 {
            return Err("expression tag must be ≤24 characters".into());
        }
        if !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return Err("expression tag may only contain letters, numbers, '-' or '_'".into());
        }
        if !out.contains(&trimmed) {
            out.push(trimmed);
        }
    }
    Ok(out)
}

fn validate_emoji_bytes(bytes: &[u8], animated: bool) -> Result<(), String> {
    let max_len = if animated {
        MAX_ANIMATED_EMOJI_BYTES
    } else {
        MAX_STATIC_EMOJI_BYTES
    };
    if bytes.is_empty() {
        return Err("emoji upload cannot be empty".into());
    }
    if bytes.len() > max_len {
        return Err(format!("emoji upload exceeds {}KB limit", max_len / 1024));
    }

    match detect_media_type(bytes, animated) {
        Some(_) => Ok(()),
        None if animated => Err("animated emoji must be PNG, WebP, or GIF".into()),
        None => Err("emoji must be PNG or WebP".into()),
    }
}

fn validate_sticker_bytes(bytes: &[u8], animated: bool) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("sticker upload cannot be empty".into());
    }
    if bytes.len() > MAX_STICKER_BYTES {
        return Err(format!(
            "sticker upload exceeds {}KB limit",
            MAX_STICKER_BYTES / 1024
        ));
    }
    match detect_media_type(bytes, animated) {
        Some(_) => Ok(()),
        None if animated => Err("animated sticker must be PNG, WebP, or GIF".into()),
        None => Err("sticker must be PNG or WebP".into()),
    }
}

fn validate_soundboard_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("soundboard upload cannot be empty".into());
    }
    if bytes.len() > MAX_SOUNDBOARD_BYTES {
        return Err(format!(
            "soundboard upload exceeds {}KB limit",
            MAX_SOUNDBOARD_BYTES / 1024
        ));
    }
    if detect_audio_kind(bytes).is_none() {
        return Err("soundboard sound must be Opus (OGG/WebM) or MP3".into());
    }
    Ok(())
}

/// Sniff a soundboard payload's container so we don't accept arbitrary
/// bytes as audio. Architecture §18.3 keeps clips small (Opus or MP3).
fn detect_audio_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"OggS") {
        return Some("audio/ogg");
    }
    if bytes.len() >= 4 && bytes.starts_with(b"\x1A\x45\xDF\xA3") {
        // EBML header — Opus-in-WebM container.
        return Some("audio/webm");
    }
    if bytes.starts_with(b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
    {
        return Some("audio/mpeg");
    }
    None
}

fn detect_media_type(bytes: &[u8], animated: bool) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if animated && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some("image/gif");
    }
    None
}

fn random_16_bytes() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}
