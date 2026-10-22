//! The merged governance snapshot a community restores from at the next
//! login: the community row's governance fields and its channels, roles,
//! categories and permission overwrites, replaced together.

use rusqlite::{params, Connection};

/// A channel as merged governance names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotChannel {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub sort_order: i64,
    pub category_id: Option<String>,
    pub topic: String,
    pub slowmode_seconds: i64,
    pub nsfw: bool,
    /// The channel's message record (also its log spine key).
    pub record_key: Option<String>,
    pub mek_generation: i64,
    pub my_sequence: i64,
}

/// A role definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRole {
    pub role_id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: u32,
    pub hoist: bool,
    pub mentionable: bool,
    pub self_assignable: bool,
    pub exclusion_group: Option<String>,
}

/// A channel category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotCategory {
    pub id: String,
    pub name: String,
    pub sort_order: i64,
}

/// A channel permission overwrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotOverwrite {
    pub channel_id: String,
    pub target_type: String,
    pub target_id: String,
    pub allow: u64,
    pub deny: u64,
}

/// Everything one merge decides for a community.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GovernanceSnapshot {
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub banner_hash: Option<String>,
    pub my_role_ids: Vec<u32>,
    pub mek_generation: i64,
    pub lamport_clock: i64,
    pub channels: Vec<SnapshotChannel>,
    pub roles: Vec<SnapshotRole>,
    pub categories: Vec<SnapshotCategory>,
    pub overwrites: Vec<SnapshotOverwrite>,
}

/// Write the snapshot: the community row's governance fields (the clock
/// only advances) and the community's channels, roles, categories and
/// overwrites replaced by the snapshot's.
///
/// # Errors
/// A write failed.
pub fn replace(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    snapshot: &GovernanceSnapshot,
) -> rusqlite::Result<()> {
    super::apply_governance(
        conn,
        owner_key,
        community_id,
        &super::GovernanceFields {
            name: &snapshot.name,
            description: snapshot.description.as_deref(),
            icon_hash: snapshot.icon_hash.as_deref(),
            banner_hash: snapshot.banner_hash.as_deref(),
            my_role_ids: &snapshot.my_role_ids,
            mek_generation: snapshot.mek_generation,
            lamport_clock: snapshot.lamport_clock,
        },
    )?;
    replace_channels(conn, owner_key, community_id, &snapshot.channels)?;
    replace_roles(conn, owner_key, community_id, &snapshot.roles)?;
    replace_categories(conn, owner_key, community_id, &snapshot.categories)?;
    replace_overwrites(conn, owner_key, community_id, &snapshot.overwrites)
}

fn replace_channels(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channels: &[SnapshotChannel],
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM channels WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
    )?;
    for c in channels {
        conn.execute(
            "INSERT INTO channels (owner_key, id, community_id, name, channel_type, sort_order, \
             category_id, topic, slowmode_seconds, nsfw, message_record_key, mek_generation, \
             log_key, my_sequence) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?11, ?13)",
            params![
                owner_key,
                c.id,
                community_id,
                c.name,
                c.channel_type,
                c.sort_order,
                c.category_id,
                c.topic,
                c.slowmode_seconds,
                i32::from(c.nsfw),
                c.record_key,
                c.mek_generation,
                c.my_sequence,
            ],
        )?;
    }
    Ok(())
}

fn replace_roles(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    roles: &[SnapshotRole],
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM community_roles WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
    )?;
    for r in roles {
        conn.execute(
            "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, \
             permissions, position, hoist, mentionable, self_assignable, exclusion_group) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                owner_key,
                community_id,
                i64::from(r.role_id),
                r.name,
                i64::from(r.color),
                r.permissions.cast_signed(),
                i64::from(r.position),
                i32::from(r.hoist),
                i32::from(r.mentionable),
                i32::from(r.self_assignable),
                r.exclusion_group,
            ],
        )?;
    }
    Ok(())
}

fn replace_categories(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    categories: &[SnapshotCategory],
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM community_categories WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
    )?;
    for c in categories {
        conn.execute(
            "INSERT INTO community_categories (owner_key, community_id, id, name, sort_order) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![owner_key, community_id, c.id, c.name, c.sort_order],
        )?;
    }
    Ok(())
}

fn replace_overwrites(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    overwrites: &[SnapshotOverwrite],
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM channel_overwrites WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
    )?;
    for o in overwrites {
        conn.execute(
            "INSERT INTO channel_overwrites \
             (owner_key, community_id, channel_id, target_type, target_id, allow, deny) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                owner_key,
                community_id,
                o.channel_id,
                o.target_type,
                o.target_id,
                o.allow.cast_signed(),
                o.deny.cast_signed(),
            ],
        )?;
    }
    Ok(())
}
