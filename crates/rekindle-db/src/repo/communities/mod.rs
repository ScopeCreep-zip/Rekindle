//! `communities`: one row per community the identity is in.

use rusqlite::{params, Connection, OptionalExtension as _};

pub mod load;
pub mod snapshot;

#[cfg(test)]
mod beat_seq_tests;

/// A community as login loads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub banner_hash: Option<String>,
    pub my_pseudonym_key: Option<String>,
    pub mek_generation: u64,
    pub member_registry_key: Option<String>,
    pub my_subkey_index: Option<u32>,
    pub my_segment_index: Option<u32>,
    /// Our own member row's onboarding flag.
    pub onboarding_complete: bool,
    /// `None` when the community never set one.
    pub presence_policy: Option<rekindle_types::presence::PresenceSharingPolicy>,
    /// Highest Lamport timestamp among the community's stored channel
    /// messages: where the message clock resumes.
    pub message_clock: u64,
    /// The remover's display name as known when our removal was first
    /// merged (plan E3.2 S9, A/B R5-2); `None` while we are a member.
    pub removed_by_name: Option<String>,
    /// Our own member record's key, as the join or create stored it.
    pub my_member_record: Option<String>,
    /// Our leave took effect locally; its `MemberLeft` is still to sign
    /// (plan E3.2 S7d-keys.8).
    pub departing: bool,
}

/// Every community of `owner_key`.
///
/// # Errors
/// The query failed.
pub fn load_all(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<CommunityRow>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.description, c.icon_hash, c.banner_hash, \
         c.my_pseudonym_key, c.mek_generation, \
         c.member_registry_key, c.my_subkey_index, c.my_segment_index, \
         COALESCE(cm.onboarding_complete, 0), c.presence_policy, \
         (SELECT COALESCE(MAX(m.lamport_ts), 0) FROM messages m \
            JOIN channels ch ON ch.owner_key = m.owner_key AND ch.id = m.conversation_id \
           WHERE m.owner_key = c.owner_key AND m.conversation_type = 'channel' \
             AND ch.community_id = c.id), \
         c.removed_by_name, c.my_member_record, c.departing \
         FROM communities c \
         LEFT JOIN community_members cm \
           ON cm.owner_key = c.owner_key \
          AND cm.community_id = c.id \
          AND cm.pseudonym_key = c.my_pseudonym_key \
         WHERE c.owner_key = ?1",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok(CommunityRow {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                icon_hash: row.get(3)?,
                banner_hash: row.get(4)?,
                my_pseudonym_key: row.get(5)?,
                mek_generation: crate::row::get_u64(row, 6)?,
                member_registry_key: row.get(7)?,
                my_subkey_index: row.get(8)?,
                my_segment_index: row.get(9)?,
                onboarding_complete: row.get::<_, i64>(10)? != 0,
                presence_policy: crate::row::get_json_opt(row, "presence_policy")?,
                message_clock: crate::row::get_u64(row, 12)?,
                removed_by_name: row.get(13)?,
                my_member_record: row.get(14)?,
                departing: row.get::<_, i64>(15)? != 0,
            })
        })?
        .collect();
    rows
}

/// A community being created or joined.
#[derive(Debug, Clone, Copy)]
pub struct NewCommunity<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub joined_at: i64,
    pub my_pseudonym_key: &'a str,
    pub mek_generation: i64,
    pub member_registry_key: Option<&'a str>,
    pub my_subkey_index: Option<u32>,
}

fn insert_with(
    conn: &Connection,
    verb: &str,
    on_conflict: &str,
    owner_key: &str,
    c: &NewCommunity<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "{verb} INTO communities (owner_key, id, name, joined_at, \
             my_pseudonym_key, mek_generation, member_registry_key, \
             my_subkey_index, my_segment_index) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0){on_conflict}"
        ),
        params![
            owner_key,
            c.id,
            c.name,
            c.joined_at,
            c.my_pseudonym_key,
            c.mek_generation,
            c.member_registry_key,
            c.my_subkey_index,
        ],
    )?;
    Ok(())
}

/// Store a community we created.
///
/// # Errors
/// The write failed (the community is already stored).
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    community: &NewCommunity<'_>,
) -> rusqlite::Result<()> {
    insert_with(conn, "INSERT", "", owner_key, community)
}

/// Store a community we joined, unless it is already stored.
///
/// # Errors
/// The write failed.
pub fn insert_if_absent(
    conn: &Connection,
    owner_key: &str,
    community: &NewCommunity<'_>,
) -> rusqlite::Result<()> {
    insert_with(conn, "INSERT OR IGNORE", "", owner_key, community)
}

/// Store a membership: a new row, or over the row a removal kept as
/// read-only history (plan E3.2 S9), whose membership columns the re-join
/// replaces and whose removal it clears, as a Matrix re-join's
/// `m.room.member` join supersedes the leave. The kept history (its
/// description, channels and messages, the clocks) stays, as Discord keeps
/// a re-joined server's messages.
///
/// # Errors
/// The write failed.
pub fn upsert_membership(
    conn: &Connection,
    owner_key: &str,
    community: &NewCommunity<'_>,
) -> rusqlite::Result<()> {
    insert_with(
        conn,
        "INSERT",
        " ON CONFLICT (owner_key, id) DO UPDATE SET \
         name = excluded.name, joined_at = excluded.joined_at, \
         my_pseudonym_key = excluded.my_pseudonym_key, \
         mek_generation = excluded.mek_generation, \
         member_registry_key = excluded.member_registry_key, \
         my_subkey_index = excluded.my_subkey_index, \
         my_segment_index = excluded.my_segment_index, \
         removed_by_name = NULL",
        owner_key,
        community,
    )
}

/// Delete the community's row.
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, owner_key: &str, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM communities WHERE owner_key = ?1 AND id = ?2",
        params![owner_key, id],
    )?;
    Ok(())
}

/// A column [`set`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Description,
    IconHash,
    BannerHash,
    PresencePolicy,
    MemberRegistryKey,
    MySubkeyIndex,
    MySegmentIndex,
    MekGeneration,
    /// The pinned `GenesisCert` as the anchor holds it (plan E3.2):
    /// `rekindle_codec::governance_op::genesis::encode_genesis`.
    GenesisCert,
    /// The remover's display name, kept with our own removal (A/B R5-2).
    RemovedByName,
    /// Our own member record's key (plan E3.2, ADR 0011).
    MyMemberRecord,
    /// Our leave took effect locally, its `MemberLeft` still to sign
    /// (plan E3.2 S7d-keys.8): 1, never cleared (the row is deleted).
    Departing,
}

impl Column {
    fn name(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Description => "description",
            Self::IconHash => "icon_hash",
            Self::BannerHash => "banner_hash",
            Self::PresencePolicy => "presence_policy",
            Self::MemberRegistryKey => "member_registry_key",
            Self::MySubkeyIndex => "my_subkey_index",
            Self::MySegmentIndex => "my_segment_index",
            Self::MekGeneration => "mek_generation",
            Self::GenesisCert => "genesis_cert",
            Self::RemovedByName => "removed_by_name",
            Self::MyMemberRecord => "my_member_record",
            Self::Departing => "departing",
        }
    }
}

/// Set one column of the community's row.
///
/// # Errors
/// The write failed.
pub fn set(
    conn: &Connection,
    owner_key: &str,
    id: &str,
    column: Column,
    value: impl rusqlite::ToSql,
) -> rusqlite::Result<()> {
    update_column(conn, owner_key, id, column, value).map(drop)
}

fn update_column(
    conn: &Connection,
    owner_key: &str,
    id: &str,
    column: Column,
    value: impl rusqlite::ToSql,
) -> rusqlite::Result<usize> {
    conn.execute(
        &format!(
            "UPDATE communities SET {} = ?1 WHERE owner_key = ?2 AND id = ?3",
            column.name()
        ),
        params![value, owner_key, id],
    )
}

/// Persist our presence consent on a joined community's row.
///
/// # Errors
/// The policy could not serialize, the row was missing, or the write failed.
pub fn set_presence_policy(
    conn: &Connection,
    owner_key: &str,
    id: &str,
    policy: &rekindle_types::presence::PresenceSharingPolicy,
) -> rusqlite::Result<()> {
    let json = serde_json::to_string(policy)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let changed = update_column(conn, owner_key, id, Column::PresencePolicy, json)?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

/// The community's pinned `GenesisCert` bytes; `None` when the row or the
/// cert is absent.
///
/// # Errors
/// The query failed.
pub fn genesis_cert(
    conn: &Connection,
    owner_key: &str,
    id: &str,
) -> rusqlite::Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            "SELECT genesis_cert FROM communities WHERE owner_key = ?1 AND id = ?2",
            params![owner_key, id],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten())
}

/// Advance our beat count in the community by one and return it
/// (`beat_seq`, plan E3.4 S1): persisted before the beat it numbers is
/// written, so no beat `seq` is reused across a restart.
///
/// # Errors
/// No row for the community, or the write failed.
pub fn next_beat_seq(conn: &Connection, owner_key: &str, id: &str) -> rusqlite::Result<u64> {
    let seq: i64 = conn.query_row(
        "UPDATE communities SET beat_seq = beat_seq + 1 \
         WHERE owner_key = ?1 AND id = ?2 RETURNING beat_seq",
        params![owner_key, id],
        |row| row.get(0),
    )?;
    u64::try_from(seq).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, seq))
}

/// The community fields merged governance decides.
#[derive(Debug, Clone, Copy)]
pub struct GovernanceFields<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub icon_hash: Option<&'a str>,
    pub banner_hash: Option<&'a str>,
    pub mek_generation: i64,
}

/// Write the fields merged governance decides.
///
/// # Errors
/// The write failed.
pub fn apply_governance(
    conn: &Connection,
    owner_key: &str,
    id: &str,
    fields: &GovernanceFields<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE communities SET name = ?1, description = ?2, icon_hash = ?3, banner_hash = ?4, \
         mek_generation = ?5 \
         WHERE owner_key = ?6 AND id = ?7",
        params![
            fields.name,
            fields.description,
            fields.icon_hash,
            fields.banner_hash,
            fields.mek_generation,
            owner_key,
            id,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
