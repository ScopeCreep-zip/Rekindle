//! Mapping from DHT payload types to display types.

//! The `ChannelEntry` → display and `RoleEntry` → display mappers lived
//! here. Both mapped the v1.0 governance-manifest payload types, which
//! nothing has written since channels and roles became `ChannelCreated`
//! and `RoleDefinition` governance entries — the caller now builds the
//! display types from merged CRDT state directly.

use crate::crypto::mek::MekCache;
use crate::payload::dht_types::ChannelMessage;

/// Abbreviate a hex key for display: first 8 + "…" + last 4.
pub(super) fn abbreviate_key(key: &str) -> String {
    if key.len() > 12 {
        format!("{}…{}", &key[..8], &key[key.len() - 4..])
    } else {
        key.to_string()
    }
}

/// Decrypt a channel message body using the MEK cache.
///
/// Returns `(body, is_encrypted, needs_mek_generation)`.
///
/// If the MEK for the message's generation is cached, decrypts and returns
/// the UTF-8 body. If the MEK is missing, returns a placeholder body with
/// `is_encrypted = true` and `needs_mek = Some(generation)` so the caller
/// can request the MEK and retry.
pub(super) fn decrypt_channel_body(
    mek_cache: &MekCache,
    community_id: &str,
    channel_id: &str,
    msg: &ChannelMessage,
) -> (String, bool, Option<u64>) {
    if msg.ciphertext.is_empty() {
        return (String::new(), false, None);
    }

    let Some(mek) = mek_cache.get_generation(community_id, channel_id, msg.mek_generation) else {
        return (
            format!("[encrypted, MEK gen {} not cached]", msg.mek_generation),
            true,
            Some(msg.mek_generation),
        );
    };

    match mek.decrypt(&msg.ciphertext) {
        Ok(plaintext) => {
            let body = String::from_utf8_lossy(&plaintext).into_owned();
            (body, false, None)
        }
        Err(e) => {
            tracing::debug!(
                generation = msg.mek_generation,
                error = %e,
                "MEK decryption failed — key may be stale or message corrupt"
            );
            (
                format!("[decryption failed, MEK gen {}]", msg.mek_generation),
                true,
                None, // MEK exists but decryption failed — don't request again
            )
        }
    }
}
