//! Phase 23.C — row DTOs + per-table `load_*_rows` query helpers,
//! lifted from `commands/auth.rs`. The eight tables involved in the
//! login-time community load (communities, channels, roles,
//! categories, members, event_rsvps, slowmode) are loaded in a single
//! `db_call` round-trip via [`fetch_community_loader_rows`].

use crate::db;
use crate::db_helpers::db_call;
use crate::state::ChannelType;
use rekindle_db::Db;

pub use rekindle_db::repo::communities::CommunityRow;
pub use rekindle_db::repo::members::ProfileRow as MemberRow;

// ── community-load row DTOs ──
//
// Typed structs replace the long tuples that used to live in this query
// fan-out. Each one mirrors the SELECT columns exactly so query helpers
// can be small and named.

pub struct ChannelRow {
    pub id: String,
    pub community_id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub category_id: Option<String>,
    pub topic: String,
    pub slowmode_seconds: Option<u32>,
    pub nsfw: bool,
    pub message_record_key: Option<String>,
    pub mek_generation: u64,
    pub log_key: Option<String>,
    pub my_sequence: u64,
    pub notification_level: i64,
    pub notification_sound_ref: Option<String>,
    pub parent_voice_channel_id: Option<String>,
}

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

pub struct CategoryRow {
    pub community_id: String,
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

pub struct EventRsvpRow {
    pub community_id: String,
    pub event_id: String,
    pub status: String,
}

pub struct SlowmodeRow {
    pub community_id: String,
    pub channel_id: String,
    pub last_send_ms: i64,
}

pub struct CommunityLoaderRows {
    pub communities: Vec<CommunityRow>,
    pub channels: Vec<ChannelRow>,
    pub roles: Vec<RoleRow>,
    pub categories: Vec<CategoryRow>,
    pub members: Vec<MemberRow>,
    pub event_rsvps: Vec<EventRsvpRow>,
    pub slowmode: Vec<SlowmodeRow>,
}

fn load_role_rows(conn: &rusqlite::Connection, owner_key: &str) -> rusqlite::Result<Vec<RoleRow>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable, exclusion_group \
         FROM community_roles WHERE owner_key = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
        Ok(RoleRow {
            community_id: db::get_str(row, "community_id"),
            role_id: row.get::<_, u32>("role_id").unwrap_or(0),
            name: db::get_str(row, "name"),
            color: row.get::<_, u32>("color").unwrap_or(0),
            permissions: row
                .get::<_, i64>("permissions")
                .unwrap_or(0)
                .cast_unsigned(),
            position: row.get::<_, i32>("position").unwrap_or(0),
            hoist: row.get::<_, i32>("hoist").unwrap_or(0) != 0,
            mentionable: row.get::<_, i32>("mentionable").unwrap_or(0) != 0,
            self_assignable: row.get::<_, i32>("self_assignable").unwrap_or(0) != 0,
            exclusion_group: db::get_str_opt(row, "exclusion_group"),
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
}

fn load_category_rows(
    conn: &rusqlite::Connection,
    owner_key: &str,
) -> rusqlite::Result<Vec<CategoryRow>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, id, name, sort_order \
         FROM community_categories WHERE owner_key = ?1 ORDER BY sort_order",
    )?;
    let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
        Ok(CategoryRow {
            community_id: db::get_str(row, "community_id"),
            id: db::get_str(row, "id"),
            name: db::get_str(row, "name"),
            sort_order: row.get::<_, i32>("sort_order").unwrap_or(0),
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
}

fn load_channel_rows(
    conn: &rusqlite::Connection,
    owner_key: &str,
) -> rusqlite::Result<Vec<ChannelRow>> {
    let mut stmt = conn.prepare(
        "SELECT ch.id, ch.community_id, ch.name, ch.channel_type, ch.category_id, ch.topic, \
                ch.slowmode_seconds, ch.nsfw, ch.message_record_key, ch.mek_generation, \
                ch.log_key, ch.my_sequence, ch.parent_voice_channel_id, \
                COALESCE(np.level, 0) AS notification_level, \
                np.sound_ref AS notification_sound_ref \
         FROM channels ch
         LEFT JOIN notification_preferences np
           ON np.owner_key = ch.owner_key
          AND np.community_id = ch.community_id
          AND np.channel_id = ch.id
         WHERE ch.owner_key = ?1
         ORDER BY ch.sort_order",
    )?;
    let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
        Ok(ChannelRow {
            id: db::get_str(row, "id"),
            community_id: db::get_str(row, "community_id"),
            name: db::get_str(row, "name"),
            // No ToSql/FromSql impl for ChannelType itself: it's now an
            // alias for rekindle_types::channel::ChannelKind (orphan
            // rule — neither the trait nor the type is local once it's
            // not src-tauri's own enum). Read as text and parse,
            // matching the fallback convention already used at the
            // other two ChannelType parse sites in this crate.
            channel_type: row
                .get::<_, String>("channel_type")?
                .parse()
                .unwrap_or(ChannelType::Text),
            category_id: db::get_str_opt(row, "category_id"),
            topic: db::get_str(row, "topic"),
            slowmode_seconds: row
                .get::<_, Option<i64>>("slowmode_seconds")
                .unwrap_or(None)
                .map(|v| u32::try_from(v).unwrap_or(0)),
            nsfw: row.get::<_, i64>("nsfw").unwrap_or(0) != 0,
            message_record_key: db::get_str_opt(row, "message_record_key"),
            mek_generation: row
                .get::<_, i64>("mek_generation")
                .unwrap_or(0)
                .cast_unsigned(),
            log_key: db::get_str_opt(row, "log_key"),
            my_sequence: row
                .get::<_, i64>("my_sequence")
                .unwrap_or(0)
                .cast_unsigned(),
            notification_level: row.get::<_, i64>("notification_level").unwrap_or(0),
            notification_sound_ref: db::get_str_opt(row, "notification_sound_ref"),
            parent_voice_channel_id: db::get_str_opt(row, "parent_voice_channel_id"),
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
}

fn load_event_rsvp_rows(
    conn: &rusqlite::Connection,
    owner_key: &str,
) -> rusqlite::Result<Vec<EventRsvpRow>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, event_id, status FROM community_event_rsvps WHERE owner_key = ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
        Ok(EventRsvpRow {
            community_id: db::get_str(row, "community_id"),
            event_id: db::get_str(row, "event_id"),
            status: db::get_str(row, "status"),
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
}

fn load_slowmode_rows(
    conn: &rusqlite::Connection,
    owner_key: &str,
) -> rusqlite::Result<Vec<SlowmodeRow>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, channel_id, last_send_ms FROM channel_slowmode_state \
         WHERE owner_key = ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
        Ok(SlowmodeRow {
            community_id: db::get_str(row, "community_id"),
            channel_id: db::get_str(row, "channel_id"),
            last_send_ms: row.get::<_, i64>("last_send_ms").unwrap_or(0),
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
}

/// Run the seven SELECT queries in a single connection trip and return
/// row DTOs grouped by table.
pub async fn fetch_community_loader_rows(
    pool: &Db,
    owner_key: &str,
) -> Result<CommunityLoaderRows, String> {
    let ok = owner_key.to_string();
    db_call(pool, move |conn| {
        Ok(CommunityLoaderRows {
            communities: rekindle_db::repo::communities::load_all(conn, &ok)?,
            channels: load_channel_rows(conn, &ok)?,
            roles: load_role_rows(conn, &ok)?,
            categories: load_category_rows(conn, &ok)?,
            members: rekindle_db::repo::members::load_profiles(conn, &ok)?,
            event_rsvps: load_event_rsvp_rows(conn, &ok)?,
            slowmode: load_slowmode_rows(conn, &ok)?,
        })
    })
    .await
}
