//! Sealing and opening an outbox entry's body (plan E3.3): the body is
//! sealed under the current text key of its channel's `scope_for_text` at
//! its [`BodyPosition`], and the entry carries that key's generation and
//! the channel tag derived from the same key, so a reader that holds the
//! generation finds the entry's channel and opens it, and nobody else can
//! link the entry to a channel.

use rekindle_secrets::channel_body::{channel_tag, CHANNEL_TAG_LEN};
use rekindle_types::channel_keys::{self, ChannelKeyProvider, KeyEpoch, KeyScope};

use crate::error::ChannelError;
use crate::send::{decrypt_channel_body, encrypt_channel_body, BodyPosition};

/// A body sealed for an outbox entry: its ciphertext, the generation that
/// sealed it and the stream's tag under that generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedEntryBody {
    pub ciphertext: Vec<u8>,
    pub generation: u64,
    pub channel_tag: [u8; CHANNEL_TAG_LEN],
}

/// Seal `body` at `at` under the current key of `scope` (the stream's text
/// scope; `at.channel_id` is the stream the tag names).
///
/// # Errors
/// No key of `scope` is held, or the AEAD failed.
pub fn seal_entry_body(
    keys: &dyn ChannelKeyProvider,
    community_id: &str,
    scope: KeyScope,
    at: BodyPosition<'_>,
    body: &[u8],
) -> Result<SealedEntryBody, ChannelError> {
    let (epoch, key) = channel_keys::current_key(keys, community_id, scope).ok_or_else(|| {
        ChannelError::MekMissing {
            community: community_id.into(),
            channel: at.channel_id.into(),
        }
    })?;
    let ciphertext = encrypt_channel_body(&key, at, body)
        .map_err(|e| ChannelError::Encrypt(format!("outbox entry body: {e}")))?;
    Ok(SealedEntryBody {
        ciphertext,
        generation: epoch.0,
        channel_tag: channel_tag(&key, at.channel_id),
    })
}

/// The first of `streams` (stream id, text scope) whose tag under
/// `generation` is `tag`: the stream an entry belongs to, when we hold that
/// generation.
#[must_use]
pub fn stream_of_tag<'s>(
    keys: &dyn ChannelKeyProvider,
    community_id: &str,
    streams: impl IntoIterator<Item = (&'s str, KeyScope)>,
    generation: u64,
    tag: &[u8; CHANNEL_TAG_LEN],
) -> Option<&'s str> {
    streams.into_iter().find_map(|(stream, scope)| {
        let key = keys.key(community_id, scope, KeyEpoch(generation))?;
        (channel_tag(&key, stream) == *tag).then_some(stream)
    })
}

/// Open a body sealed at `at` under exactly `generation` of `scope`;
/// `None` when that generation is not held or the body does not
/// authenticate at `at`.
#[must_use]
pub fn open_entry_body(
    keys: &dyn ChannelKeyProvider,
    community_id: &str,
    scope: KeyScope,
    generation: u64,
    at: BodyPosition<'_>,
    ciphertext: &[u8],
) -> Option<Vec<u8>> {
    let key = keys.key(community_id, scope, KeyEpoch(generation))?;
    decrypt_channel_body(&key, at, ciphertext).ok()
}

#[cfg(test)]
#[path = "outbox_seal_tests.rs"]
mod tests;
