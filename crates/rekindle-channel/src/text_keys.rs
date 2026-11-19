//! The text plane's keys: channel messages, threads and their bodies are
//! under the parent channel's `scope_for_text` (plan D6), sealed with the
//! shared `rekindle_secrets::channel_body` codec, and named by the exact
//! generation of the key that sealed them.

use rekindle_types::channel_keys::{self, KeyEpoch, KeyScope, Zeroizing};
use rekindle_types::id::ChannelId;

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;
use crate::send::{decrypt_channel_body, At};

fn text_scope<D: ChannelMessagingDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<KeyScope, ChannelError> {
    let channel = ChannelId::from_hex(channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    Ok(deps.keys().scope_for_text(community_id, channel))
}

/// Whether we hold a text key of `channel_id` (the parent channel, for a
/// thread) to seal a post under (plan E3.2 S7l).
///
/// # Errors
/// `channel_id` is not a channel id.
pub(crate) fn holds_text_key<D: ChannelMessagingDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<bool, ChannelError> {
    let scope = text_scope(deps, community_id, channel_id)?;
    Ok(channel_keys::current_key(&*deps.keys(), community_id, scope).is_some())
}

/// Open a text body sealed at `at` under exactly `generation` of
/// `channel_id`'s text scope. `None` when that generation is not held or
/// the body does not authenticate at `at`.
pub(crate) fn open_text<'a, D: ChannelMessagingDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    generation: u64,
    at: impl Into<At<'a>>,
    ciphertext: &[u8],
) -> Option<Vec<u8>> {
    let scope = text_scope(deps, community_id, channel_id).ok()?;
    let key: Zeroizing<[u8; 32]> = deps.keys().key(community_id, scope, KeyEpoch(generation))?;
    decrypt_channel_body(&key, at, ciphertext).ok()
}
