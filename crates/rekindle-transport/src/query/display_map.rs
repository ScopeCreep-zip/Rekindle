//! Mapping from DHT payload types to display types.

use crate::crypto::mek::MekCache;
use crate::payload::dht_types::{ChannelEntry, ChannelKind, ChannelMessage, RoleEntry};

use super::{ChannelOverviewDisplay, RoleDisplay};

// ── Free functions ──────────────────────────────────────────────────────

pub(super) fn channel_to_display(entry: &ChannelEntry) -> ChannelOverviewDisplay {
    ChannelOverviewDisplay {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: channel_kind_str(entry.kind),
        category_id: entry.category_id.clone(),
        topic: entry.topic.clone(),
        mek_generation: entry.mek_generation,
        log_key: entry.log_key.clone(),
        sort_order: entry.sort_order,
    }
}

pub(super) fn channel_kind_str(kind: ChannelKind) -> String {
    match kind {
        ChannelKind::Text => "text",
        ChannelKind::Voice => "voice",
        ChannelKind::Announcement => "announcement",
        ChannelKind::Forum => "forum",
        ChannelKind::Stage => "stage",
        ChannelKind::Directory => "directory",
        ChannelKind::Media => "media",
        ChannelKind::Events => "events",
        ChannelKind::Dm => "dm",
    }
    .to_string()
}

pub(super) fn role_to_display(entry: &RoleEntry) -> RoleDisplay {
    RoleDisplay {
        id: entry.id,
        name: entry.name.clone(),
        color: entry.color,
        permissions: entry.permissions,
        position: entry.position,
    }
}

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
