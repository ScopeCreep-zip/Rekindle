//! Phase 23.D.3 — per-channel + community-default notification sound
//! setter + resolver. Sound refs are BLAKE3 content hashes pointing
//! to soundboard expression assets in the Lost Cargo cache.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

/// Architecture §32 Phase 7 Week 25 — set the notification sound for
/// `(community_id, channel_id)`. Pass `channel_id = ""` to set the
/// community-wide default. `sound_ref = None` removes the override and
/// re-inherits from the next level up.
pub async fn set_notification_sound(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    sound_ref: Option<String>,
) -> Result<(), String> {
    // Mirror the channel-level setting into in-memory `ChannelInfo` so
    // `get_community_details` returns the up-to-date value without an
    // extra DB round-trip. Empty `channel_id` means "community default"
    // and is not stored on any per-channel row.
    if !channel_id.is_empty() {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(channel) = community
                .channels
                .iter_mut()
                .find(|channel| channel.id == channel_id)
            {
                channel.notification_sound_ref.clone_from(&sound_ref);
            }
        }
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id_owned = community_id.to_string();
    let channel_id_owned = channel_id.to_string();
    db_call(pool, move |conn| {
        // Upsert: notification_preferences may already have a row from
        // set_channel_notification_level. We update sound_ref without
        // disturbing the level column.
        conn.execute(
            "INSERT INTO notification_preferences \
                  (owner_key, community_id, channel_id, level, sound_ref) \
             VALUES (?1, ?2, ?3, 0, ?4) \
             ON CONFLICT(owner_key, community_id, channel_id) \
             DO UPDATE SET sound_ref = excluded.sound_ref",
            rusqlite::params![owner_key, community_id_owned, channel_id_owned, sound_ref],
        )?;
        Ok(())
    })
    .await
}

/// Three-tier resolver mirroring `resolve_notification_level`:
/// channel override → community default → `None` (caller falls back to
/// the app-global `notification_sound: bool` toggle in `app_settings`).
pub async fn resolve_notification_sound(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
) -> Option<String> {
    let owner = owner_key.to_string();
    let cid = community_id.to_string();
    let chid = channel_id.to_string();
    let row: Option<String> = crate::db_helpers::db_call_or_default(pool, move |conn| {
        // Channel override.
        let channel: Option<String> = conn
            .query_row(
                "SELECT sound_ref FROM notification_preferences \
                  WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ?3",
                rusqlite::params![owner, cid, chid],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if channel.is_some() {
            return Ok(channel);
        }
        // Community default (channel_id == '').
        let default: Option<String> = conn
            .query_row(
                "SELECT sound_ref FROM notification_preferences \
                  WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ''",
                rusqlite::params![owner, cid],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        Ok(default)
    })
    .await;
    row
}
