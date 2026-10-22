//! The unlock's warm load of every joined community (plan E3.1 C1): the
//! `communities` rows with their channels, roles, categories, member
//! profiles, our RSVPs and slowmode stamps, and the governance cache, in
//! one connection call, for the Host's community store.

use rusqlite::{params, Connection};

use crate::row::{get, get_enum};
use rekindle_types::channel::ChannelKind;

use super::CommunityRow;
use crate::repo::governance_cache;
use crate::repo::members::ProfileRow;

/// A channel as the warm load reads it, with our notification preference.
pub struct ChannelRow {
    pub id: String,
    pub community_id: String,
    pub name: String,
    pub channel_type: ChannelKind,
    pub category_id: Option<String>,
    pub topic: String,
    pub slowmode_seconds: Option<u32>,
    pub nsfw: bool,
    pub message_record_key: Option<String>,
    pub mek_generation: u64,
    pub log_key: Option<String>,
    pub my_sequence: u64,
    /// `0` all, `1` mentions, `2` nothing.
    pub notification_level: i64,
    pub notification_sound_ref: Option<String>,
    pub parent_voice_channel_id: Option<String>,
}

/// A stored role definition.
pub struct RoleRow {
    pub community_id: String,
    pub role_id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    pub self_assignable: bool,
    pub exclusion_group: Option<String>,
}

/// A stored channel category.
pub struct CategoryRow {
    pub community_id: String,
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// Our RSVP to one event.
pub struct EventRsvpRow {
    pub community_id: String,
    pub event_id: String,
    pub status: String,
}

/// When we last posted in a slowmode channel.
pub struct SlowmodeRow {
    pub community_id: String,
    pub channel_id: String,
    pub last_send_ms: i64,
}

/// Everything the warm load reads, grouped by table.
pub struct CommunityLoad {
    pub communities: Vec<CommunityRow>,
    pub channels: Vec<ChannelRow>,
    pub roles: Vec<RoleRow>,
    pub categories: Vec<CategoryRow>,
    pub members: Vec<ProfileRow>,
    pub event_rsvps: Vec<EventRsvpRow>,
    pub slowmode: Vec<SlowmodeRow>,
    /// The governance cache's merge input per community.
    pub governance: Vec<(String, governance_cache::Entries)>,
}

/// Read every joined community of `owner_key`.
///
/// # Errors
/// A query failed.
pub fn load(conn: &Connection, owner_key: &str) -> rusqlite::Result<CommunityLoad> {
    Ok(CommunityLoad {
        communities: super::load_all(conn, owner_key)?,
        channels: channels(conn, owner_key)?,
        roles: roles(conn, owner_key)?,
        categories: categories(conn, owner_key)?,
        members: crate::repo::members::load_profiles(conn, owner_key)?,
        event_rsvps: event_rsvps(conn, owner_key)?,
        slowmode: slowmode(conn, owner_key)?,
        governance: governance_cache::load(conn, owner_key)?,
    })
}

fn roles(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<RoleRow>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, role_id, name, color, permissions, position, hoist, mentionable, \
                self_assignable, exclusion_group \
         FROM community_roles WHERE owner_key = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok(RoleRow {
            community_id: get(row, "community_id")?,
            role_id: get(row, "role_id")?,
            name: get(row, "name")?,
            color: get(row, "color")?,
            permissions: get::<i64>(row, "permissions")?.cast_unsigned(),
            position: get(row, "position")?,
            hoist: get::<i32>(row, "hoist")? != 0,
            mentionable: get::<i32>(row, "mentionable")? != 0,
            self_assignable: get::<i32>(row, "self_assignable")? != 0,
            exclusion_group: get(row, "exclusion_group")?,
        })
    })?;
    rows.collect()
}

fn categories(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<CategoryRow>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, id, name, sort_order \
         FROM community_categories WHERE owner_key = ?1 ORDER BY sort_order",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok(CategoryRow {
            community_id: get(row, "community_id")?,
            id: get(row, "id")?,
            name: get(row, "name")?,
            sort_order: get(row, "sort_order")?,
        })
    })?;
    rows.collect()
}

fn channels(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<ChannelRow>> {
    let mut stmt = conn.prepare(
        "SELECT ch.id, ch.community_id, ch.name, ch.channel_type, ch.category_id, ch.topic, \
                ch.slowmode_seconds, ch.nsfw, ch.message_record_key, ch.mek_generation, \
                ch.log_key, ch.my_sequence, ch.parent_voice_channel_id, \
                COALESCE(np.level, 0) AS notification_level, \
                np.sound_ref AS notification_sound_ref \
         FROM channels ch \
         LEFT JOIN notification_preferences np \
           ON np.owner_key = ch.owner_key \
          AND np.community_id = ch.community_id \
          AND np.channel_id = ch.id \
         WHERE ch.owner_key = ?1 \
         ORDER BY ch.sort_order",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok(ChannelRow {
            id: get(row, "id")?,
            community_id: get(row, "community_id")?,
            name: get(row, "name")?,
            // Read as text and parsed; a value outside the set is an error.
            channel_type: get_enum::<ChannelKind>(row, "channel_type")?,
            category_id: get(row, "category_id")?,
            topic: get(row, "topic")?,
            slowmode_seconds: get(row, "slowmode_seconds")?,
            nsfw: get::<i64>(row, "nsfw")? != 0,
            message_record_key: get(row, "message_record_key")?,
            mek_generation: get::<i64>(row, "mek_generation")?.cast_unsigned(),
            log_key: get(row, "log_key")?,
            my_sequence: get::<i64>(row, "my_sequence")?.cast_unsigned(),
            notification_level: get(row, "notification_level")?,
            notification_sound_ref: get(row, "notification_sound_ref")?,
            parent_voice_channel_id: get(row, "parent_voice_channel_id")?,
        })
    })?;
    rows.collect()
}

fn event_rsvps(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<EventRsvpRow>> {
    let mut stmt = conn.prepare(
        "SELECT r.community_id, r.event_id, r.status FROM event_rsvps r \
         JOIN communities c ON c.owner_key = r.owner_key AND c.id = r.community_id \
         WHERE r.owner_key = ?1 AND r.pseudonym_key = c.my_pseudonym_key",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok(EventRsvpRow {
            community_id: get(row, "community_id")?,
            event_id: get(row, "event_id")?,
            status: get(row, "status")?,
        })
    })?;
    rows.collect()
}

fn slowmode(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<SlowmodeRow>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, channel_id, last_send_ms FROM channel_slowmode_state \
         WHERE owner_key = ?1",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok(SlowmodeRow {
            community_id: get(row, "community_id")?,
            channel_id: get(row, "channel_id")?,
            last_send_ms: get(row, "last_send_ms")?,
        })
    })?;
    rows.collect()
}
