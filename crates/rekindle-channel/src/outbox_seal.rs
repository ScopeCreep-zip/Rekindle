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
mod tests {
    use super::*;
    use rekindle_types::channel_keys::Zeroizing;
    use rekindle_types::id::ChannelId;

    /// Generations 1 and 2 of the community key.
    struct Keys;

    impl ChannelKeyProvider for Keys {
        fn current_epoch(&self, _: &str, _: KeyScope) -> Option<KeyEpoch> {
            Some(KeyEpoch(2))
        }
        fn key(&self, _: &str, _: KeyScope, epoch: KeyEpoch) -> Option<Zeroizing<[u8; 32]>> {
            let byte = u8::try_from(epoch.0).ok().filter(|g| (1..=2).contains(g))?;
            Some(Zeroizing::new([byte; 32]))
        }
        fn scope_for_text(&self, _: &str, _: ChannelId) -> KeyScope {
            KeyScope::Community
        }
    }

    const AUTHOR: [u8; 32] = [4; 32];

    fn at(channel_id: &str, stream_seq: u64) -> BodyPosition<'_> {
        BodyPosition {
            anchor: "VLD0:anchor",
            channel_id,
            author: &AUTHOR,
            stream_seq,
        }
    }

    /// A reader finds the entry's channel by its tag and opens the body at
    /// the entry's position; at another position it does not open.
    #[test]
    fn a_sealed_body_names_its_channel_and_opens_only_in_place() {
        let sealed =
            seal_entry_body(&Keys, "c", KeyScope::Community, at("chan-b", 7), b"hi").unwrap();
        assert_eq!(sealed.generation, 2);
        let streams = [
            ("chan-a", KeyScope::Community),
            ("chan-b", KeyScope::Community),
        ];
        let found = stream_of_tag(&Keys, "c", streams, sealed.generation, &sealed.channel_tag);
        assert_eq!(found, Some("chan-b"));
        assert_eq!(
            stream_of_tag(&Keys, "c", streams, 1, &sealed.channel_tag),
            None
        );
        let open = |at| open_entry_body(&Keys, "c", KeyScope::Community, 2, at, &sealed.ciphertext);
        assert_eq!(open(at("chan-b", 7)).as_deref(), Some(&b"hi"[..]));
        assert_eq!(open(at("chan-b", 8)), None);
        assert_eq!(open(at("chan-a", 7)), None);
    }
}
