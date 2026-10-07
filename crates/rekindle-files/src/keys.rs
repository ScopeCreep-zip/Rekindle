//! The keys file sharing seals under (plan D6). An attachment belongs to
//! its channel's text plane (`scope_for_text`); an expression to the
//! community. Every FEK wrap and carrying-message body names the exact
//! generation of the key that sealed it, and is opened under exactly that
//! generation — current or historical — or not at all.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::channel_keys::{self, KeyEpoch, KeyScope};
use rekindle_types::id::ChannelId;

use crate::deps::FilesDeps;
use crate::error::FilesError;

fn text_scope<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<KeyScope, FilesError> {
    let channel = ChannelId::from_hex(channel_id)
        .ok_or_else(|| FilesError::InvalidInput(format!("not a channel id: {channel_id}")))?;
    Ok(deps.keys().scope_for_text(community_id, channel))
}

fn current<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
) -> Result<MediaEncryptionKey, FilesError> {
    let (epoch, key) = channel_keys::current_key(&*deps.keys(), community_id, scope).ok_or(
        FilesError::MekUnavailable {
            community: community_id.to_string(),
            generation: 0,
        },
    )?;
    Ok(MediaEncryptionKey::from_bytes(*key, epoch.0))
}

fn at<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
    generation: u64,
) -> Result<MediaEncryptionKey, FilesError> {
    let key = deps
        .keys()
        .key(community_id, scope, KeyEpoch(generation))
        .ok_or(FilesError::MekUnavailable {
            community: community_id.to_string(),
            generation,
        })?;
    Ok(MediaEncryptionKey::from_bytes(*key, generation))
}

/// The current text key of `channel_id`, carrying its generation.
pub(crate) fn current_text_key<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<MediaEncryptionKey, FilesError> {
    current(
        deps,
        community_id,
        text_scope(deps, community_id, channel_id)?,
    )
}

/// `channel_id`'s text key at exactly `generation`.
pub(crate) fn text_key_at<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) -> Result<MediaEncryptionKey, FilesError> {
    at(
        deps,
        community_id,
        text_scope(deps, community_id, channel_id)?,
        generation,
    )
}

/// The current community key, carrying its generation.
pub(crate) fn current_community_key<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
) -> Result<MediaEncryptionKey, FilesError> {
    current(deps, community_id, KeyScope::Community)
}

/// The community key at exactly `generation`.
pub(crate) fn community_key_at<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    generation: u64,
) -> Result<MediaEncryptionKey, FilesError> {
    at(deps, community_id, KeyScope::Community, generation)
}
