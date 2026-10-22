//! Expression uploads — emoji, stickers, soundboard sounds.

use super::image::{prepare_upload, ExpressionPicture};
use super::limits::{
    normalize_tags, validate_expression_name, validate_soundboard_bytes, MAX_ANIMATED_EMOJI_COUNT,
    MAX_SOUNDBOARD_COUNT, MAX_STATIC_EMOJI_COUNT, MAX_STICKER_COUNT,
};
use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

use rekindle_types::expression::SoundboardMeta;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::image_kind::ImageKind;

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

pub(super) fn next_lamport<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
) -> Result<u64, ChannelError> {
    if deps.governance_state(community_id).is_none() {
        return Err(ChannelError::Adapter(
            "governance state not loaded for this community".into(),
        ));
    }
    Ok(deps.next_governance_lamport(community_id)?)
}

/// Upload a custom emoji. `kind` is what the file picker declared; the
/// picture is decoded as that kind only and re-encoded without metadata
/// (a GIF stays animated, PNG and JPEG become a PNG).
pub async fn upload_emoji<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    kind: ImageKind,
) -> Result<String, ChannelError> {
    validate_expression_name(name)?;
    let picture = prepare_upload(&bytes, kind, ExpressionPicture::Emoji)?;
    let animated = picture.animated();
    let (max, kind_label) = if animated {
        (MAX_ANIMATED_EMOJI_COUNT, "animated emoji")
    } else {
        (MAX_STATIC_EMOJI_COUNT, "static emoji")
    };
    enforce_count_limit(deps, community_id, "emoji", animated, max, kind_label)?;

    let expression_id = rekindle_utils::random::id_bytes_16();
    let content_hash = blake3::hash(&picture.bytes).to_hex().to_string();
    let creator = my_pseudonym_for_community(deps, community_id);
    let attachment = deps.upload_expression_to_cache(
        community_id,
        expression_id,
        &picture.bytes,
        format!("{name}.{}", picture.extension()),
        picture.kind.mime().to_string(),
    )?;

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

/// Phase 19.f — upload a custom sticker, its picture handled as
/// [`upload_emoji`]'s is.
pub async fn upload_sticker<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    kind: ImageKind,
    tags: Vec<String>,
) -> Result<String, ChannelError> {
    validate_expression_name(name)?;
    let picture = prepare_upload(&bytes, kind, ExpressionPicture::Sticker)?;
    let animated = picture.animated();
    enforce_count_limit(
        deps,
        community_id,
        "sticker",
        false,
        MAX_STICKER_COUNT,
        "sticker",
    )?;
    let normalized_tags = normalize_tags(tags)?;

    let expression_id = rekindle_utils::random::id_bytes_16();
    let content_hash = blake3::hash(&picture.bytes).to_hex().to_string();
    let creator = my_pseudonym_for_community(deps, community_id);
    let attachment = deps.upload_expression_to_cache(
        community_id,
        expression_id,
        &picture.bytes,
        format!("{name}.{}", picture.extension()),
        picture.kind.mime().to_string(),
    )?;

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

/// Inputs for a soundboard-sound upload.
///
/// Borrows the identifier strings (`&'a str`) since the orchestrator owns
/// them, and owns the moved-in payload (`bytes`, `tags`, `emoji`) that is
/// consumed by the cache write and governance entry.
pub struct UploadSoundboardSoundParams<'a> {
    pub community_id: &'a str,
    pub name: &'a str,
    pub bytes: Vec<u8>,
    pub tags: Vec<String>,
    pub duration_seconds: f32,
    pub volume: f32,
    pub emoji: Option<String>,
}

/// Phase 19.f — upload a soundboard sound.
pub async fn upload_soundboard_sound<D: ChannelMessagingDeps>(
    deps: &D,
    params: UploadSoundboardSoundParams<'_>,
) -> Result<String, ChannelError> {
    let UploadSoundboardSoundParams {
        community_id,
        name,
        bytes,
        tags,
        duration_seconds,
        volume,
        emoji,
    } = params;
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

    let expression_id = rekindle_utils::random::id_bytes_16();
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
