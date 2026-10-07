//! Mapping from DHT payload types to display types.

//! The `ChannelEntry` → display and `RoleEntry` → display mappers lived
//! here. Both mapped the v1.0 governance-manifest payload types, which
//! nothing has written since channels and roles became `ChannelCreated`
//! and `RoleDefinition` governance entries — the caller now builds the
//! display types from merged CRDT state directly.

use rekindle_secrets::channel_body::BodyPosition;
use rekindle_types::channel_keys::{ChannelKeyProvider, KeyEpoch, KeyScope};

use crate::payload::dht_types::ChannelMessage;

/// Decrypt a channel message body using the MEK cache.
///
/// Returns `(body, is_encrypted, needs_mek_generation)`.
///
/// Channel text is under the channel's text scope (plan D6) at exactly the
/// message's generation, bound to the record, subkey and Lamport position
/// it was read from. If that generation is not cached, returns a
/// placeholder with `needs_mek = Some(generation)` so the caller can
/// request it; no other key is tried.
pub(super) fn decrypt_channel_body(
    keys: &dyn ChannelKeyProvider,
    community_id: &str,
    scope: KeyScope,
    channel_record_key: &str,
    subkey_index: u32,
    msg: &ChannelMessage,
) -> (String, bool, Option<u64>) {
    if msg.ciphertext.is_empty() {
        return (String::new(), false, None);
    }

    let Some(key) = keys.key(community_id, scope, KeyEpoch(msg.mek_generation)) else {
        return (
            format!("[encrypted, MEK gen {} not cached]", msg.mek_generation),
            true,
            Some(msg.mek_generation),
        );
    };

    let at = BodyPosition {
        channel_record_key,
        subkey_index,
        lamport_ts: msg.lamport_ts,
    };
    match rekindle_secrets::channel_body::decrypt_channel_body(&key, at, &msg.ciphertext) {
        Ok(plaintext) => {
            let body = String::from_utf8_lossy(&plaintext).into_owned();
            (body, false, None)
        }
        Err(e) => {
            tracing::debug!(
                generation = msg.mek_generation,
                error = %e,
                "channel body failed to authenticate under its generation's key"
            );
            (
                format!("[decryption failed, MEK gen {}]", msg.mek_generation),
                true,
                None, // MEK exists but decryption failed — don't request again
            )
        }
    }
}
