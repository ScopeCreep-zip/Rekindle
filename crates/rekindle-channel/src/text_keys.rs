//! The text plane's keys: channel messages, threads and their bodies are
//! under the parent channel's `scope_for_text` (plan D6), sealed with the
//! shared `rekindle_secrets::channel_body` codec, and named by the exact
//! generation of the key that sealed them.

use rekindle_types::channel_keys::{self, KeyEpoch, KeyScope, Zeroizing};
use rekindle_types::id::ChannelId;

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;
use crate::send::{decrypt_channel_body, encrypt_channel_body, BodyPosition};

fn text_scope<D: ChannelMessagingDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<KeyScope, ChannelError> {
    let channel = ChannelId::from_hex(channel_id)
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    Ok(deps.keys().scope_for_text(community_id, channel))
}

/// Seal `body` at `at` under the current text key of `channel_id` (the
/// parent channel, for a thread). Returns the ciphertext and the
/// generation that sealed it — the generation the message must carry.
pub(crate) fn seal_text<D: ChannelMessagingDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    at: BodyPosition<'_>,
    body: &[u8],
) -> Result<(Vec<u8>, u64), ChannelError> {
    let scope = text_scope(deps, community_id, channel_id)?;
    let (epoch, key) =
        channel_keys::current_key(&*deps.keys(), community_id, scope).ok_or_else(|| {
            ChannelError::MekMissing {
                community: community_id.into(),
                channel: channel_id.into(),
            }
        })?;
    let ciphertext = encrypt_channel_body(&key, at, body)
        .map_err(|e| ChannelError::Encrypt(format!("channel body: {e}")))?;
    Ok((ciphertext, epoch.0))
}

/// Open a text body sealed at `at` under exactly `generation` of
/// `channel_id`'s text scope. `None` when that generation is not held or
/// the body does not authenticate at `at`.
pub(crate) fn open_text<D: ChannelMessagingDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    generation: u64,
    at: BodyPosition<'_>,
    ciphertext: &[u8],
) -> Option<Vec<u8>> {
    let scope = text_scope(deps, community_id, channel_id).ok()?;
    let key: Zeroizing<[u8; 32]> = deps.keys().key(community_id, scope, KeyEpoch(generation))?;
    decrypt_channel_body(&key, at, ciphertext).ok()
}
