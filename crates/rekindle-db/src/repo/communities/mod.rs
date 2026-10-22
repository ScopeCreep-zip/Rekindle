//! `communities`: one row per community the identity is in.

use rusqlite::{params, Connection, OptionalExtension as _};

use super::role_ids;

pub mod load;
pub mod snapshot;

/// A community as login loads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub banner_hash: Option<String>,
    pub my_role_ids: Vec<u32>,
    pub dht_owner_keypair: Option<String>,
    pub my_pseudonym_key: Option<String>,
    pub mek_generation: u64,
    pub member_registry_key: Option<String>,
    pub my_subkey_index: Option<u32>,
    pub my_segment_index: Option<u32>,
    /// Our own member row's onboarding flag.
    pub onboarding_complete: bool,
    /// `None` when the community never set one.
    pub presence_policy: Option<rekindle_types::presence::PresenceSharingPolicy>,
    /// Persisted governance clock (`lamport_clock`).
    pub governance_clock: u64,
    /// Highest Lamport timestamp among the community's stored channel
    /// messages: where the message clock resumes.
    pub message_clock: u64,
}

/// Every community of `owner_key`.
///
/// # Errors
/// The query failed.
pub fn load_all(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<CommunityRow>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.description, c.icon_hash, c.banner_hash, \
         c.my_role_ids, c.dht_owner_keypair, c.my_pseudonym_key, c.mek_generation, \
         c.member_registry_key, c.my_subkey_index, c.my_segment_index, \
         COALESCE(cm.onboarding_complete, 0), c.presence_policy, c.lamport_clock, \
         (SELECT COALESCE(MAX(m.lamport_ts), 0) FROM messages m \
            JOIN channels ch ON ch.owner_key = m.owner_key AND ch.id = m.conversation_id \
           WHERE m.owner_key = c.owner_key AND m.conversation_type = 'channel' \
             AND ch.community_id = c.id) \
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
                my_role_ids: role_ids::decode(5, &row.get::<_, String>(5)?)?,
                dht_owner_keypair: row.get(6)?,
                my_pseudonym_key: row.get(7)?,
                mek_generation: crate::row::get_u64(row, 8)?,
                member_registry_key: row.get(9)?,
                my_subkey_index: row.get(10)?,
                my_segment_index: row.get(11)?,
                onboarding_complete: row.get::<_, i64>(12)? != 0,
                presence_policy: crate::row::get_json_opt(row, "presence_policy")?,
                governance_clock: crate::row::get_u64(row, 14)?,
                message_clock: crate::row::get_u64(row, 15)?,
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
    pub my_role_ids: &'a [u32],
    pub joined_at: i64,
    pub dht_owner_keypair: Option<&'a str>,
    pub my_pseudonym_key: &'a str,
    pub mek_generation: i64,
    pub member_registry_key: Option<&'a str>,
    pub my_subkey_index: Option<u32>,
    pub governance_key: Option<&'a str>,
}

fn insert_with(
    conn: &Connection,
    verb: &str,
    owner_key: &str,
    c: &NewCommunity<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "{verb} INTO communities (owner_key, id, name, my_role_ids, joined_at, \
             dht_owner_keypair, my_pseudonym_key, mek_generation, member_registry_key, \
             my_subkey_index, my_segment_index, governance_key) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11)"
        ),
        params![
            owner_key,
            c.id,
            c.name,
            role_ids::encode(c.my_role_ids)?,
            c.joined_at,
            c.dht_owner_keypair,
            c.my_pseudonym_key,
            c.mek_generation,
            c.member_registry_key,
            c.my_subkey_index,
            c.governance_key,
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
    insert_with(conn, "INSERT", owner_key, community)
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
    insert_with(conn, "INSERT OR IGNORE", owner_key, community)
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
    DhtOwnerKeypair,
    MySubkeyIndex,
    MySegmentIndex,
    MekGeneration,
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
            Self::DhtOwnerKeypair => "dht_owner_keypair",
            Self::MySubkeyIndex => "my_subkey_index",
            Self::MySegmentIndex => "my_segment_index",
            Self::MekGeneration => "mek_generation",
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
    conn.execute(
        &format!(
            "UPDATE communities SET {} = ?1 WHERE owner_key = ?2 AND id = ?3",
            column.name()
        ),
        params![value, owner_key, id],
    )?;
    Ok(())
}

/// Our role ids in the community; `None` when it is not stored.
///
/// # Errors
/// The query failed or the stored set does not decode.
pub fn my_role_ids(
    conn: &Connection,
    owner_key: &str,
    id: &str,
) -> rusqlite::Result<Option<Vec<u32>>> {
    conn.query_row(
        "SELECT my_role_ids FROM communities WHERE owner_key = ?1 AND id = ?2",
        params![owner_key, id],
        |row| role_ids::decode(0, &row.get::<_, String>(0)?),
    )
    .optional()
}

/// Set our role ids in the community.
///
/// # Errors
/// The write failed.
pub fn set_my_role_ids(
    conn: &Connection,
    owner_key: &str,
    id: &str,
    role_ids: &[u32],
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE communities SET my_role_ids = ?1 WHERE owner_key = ?2 AND id = ?3",
        params![role_ids::encode(role_ids)?, owner_key, id],
    )?;
    Ok(())
}

/// Advance the stored governance clock; it never moves back.
///
/// # Errors
/// The write failed.
pub fn raise_clock(
    conn: &Connection,
    owner_key: &str,
    id: &str,
    clock: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE communities SET lamport_clock = MAX(lamport_clock, ?1) \
         WHERE owner_key = ?2 AND id = ?3",
        params![clock, owner_key, id],
    )?;
    Ok(())
}

/// The community fields merged governance decides.
#[derive(Debug, Clone, Copy)]
pub struct GovernanceFields<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub icon_hash: Option<&'a str>,
    pub banner_hash: Option<&'a str>,
    pub my_role_ids: &'a [u32],
    pub mek_generation: i64,
    pub lamport_clock: i64,
}

/// Write the fields merged governance decides; the clock only advances.
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
         my_role_ids = ?5, mek_generation = ?6, lamport_clock = MAX(lamport_clock, ?7) \
         WHERE owner_key = ?8 AND id = ?9",
        params![
            fields.name,
            fields.description,
            fields.icon_hash,
            fields.banner_hash,
            role_ids::encode(fields.my_role_ids)?,
            fields.mek_generation,
            fields.lamport_clock,
            owner_key,
            id,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn community<'a>(id: &'a str, roles: &'a [u32]) -> NewCommunity<'a> {
        NewCommunity {
            id,
            name: "One",
            my_role_ids: roles,
            joined_at: 0,
            dht_owner_keypair: None,
            my_pseudonym_key: "me",
            mek_generation: 3,
            member_registry_key: Some("registry"),
            my_subkey_index: Some(4),
            governance_key: None,
        }
    }

    #[test]
    fn segment_and_mek_generation_are_set() {
        let conn = fixture::conn();
        insert(&conn, OWNER, &community("c1", &[0])).unwrap();
        set(&conn, OWNER, "c1", Column::MySegmentIndex, 2).unwrap();
        set(&conn, OWNER, "c1", Column::MekGeneration, 9).unwrap();
        let row = &load_all(&conn, OWNER).unwrap()[0];
        assert_eq!(row.my_segment_index, Some(2));
        assert_eq!(row.mek_generation, 9);
    }

    /// After a restart both clocks resume where they were: governance from
    /// `lamport_clock`, messages from the community's highest stored
    /// channel `lamport_ts` (only its own channels, never DMs).
    #[test]
    fn clocks_reload_from_storage() {
        let conn = fixture::conn();
        insert(&conn, OWNER, &community("c1", &[0])).unwrap();
        insert(&conn, OWNER, &community("c2", &[0])).unwrap();
        raise_clock(&conn, OWNER, "c1", 42).unwrap();
        raise_clock(&conn, OWNER, "c1", 7).unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO channels (owner_key, id, community_id, name, channel_type) \
               VALUES ('{OWNER}', 'ch1', 'c1', 'general', 'text'), \
                      ('{OWNER}', 'ch2', 'c2', 'general', 'text'); \
             INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, lamport_ts) \
               VALUES ('{OWNER}', 'ch1', 'channel', 's', 'a', 1, 900), \
                      ('{OWNER}', 'ch1', 'channel', 's', 'b', 2, 1500), \
                      ('{OWNER}', 'ch2', 'channel', 's', 'c', 3, 7), \
                      ('{OWNER}', 'ch1', 'dm', 's', 'not a channel', 4, 99999);"
        ))
        .unwrap();
        let rows = load_all(&conn, OWNER).unwrap();
        let clocks = |id: &str| {
            let row = rows.iter().find(|r| r.id == id).unwrap();
            (row.governance_clock, row.message_clock)
        };
        assert_eq!(clocks("c1"), (42, 1500));
        assert_eq!(clocks("c2"), (0, 7));
    }

    #[test]
    fn a_joined_community_is_not_overwritten_and_columns_set_individually() {
        let conn = fixture::conn();
        insert_if_absent(&conn, OWNER, &community("c1", &[0, 1])).unwrap();
        insert_if_absent(&conn, OWNER, &community("c1", &[9])).unwrap();
        set(&conn, OWNER, "c1", Column::Description, "about").unwrap();
        set(&conn, OWNER, "c1", Column::MySubkeyIndex, 8_i64).unwrap();
        let row = &load_all(&conn, OWNER).unwrap()[0];
        assert_eq!(row.my_role_ids, [0, 1]);
        assert_eq!(row.description.as_deref(), Some("about"));
        assert_eq!(
            (
                row.my_subkey_index,
                row.my_segment_index,
                row.mek_generation
            ),
            (Some(8), Some(0), 3)
        );
        assert_eq!(my_role_ids(&conn, OWNER, "c1").unwrap(), Some(vec![0, 1]));
        set_my_role_ids(&conn, OWNER, "c1", &[2]).unwrap();
        assert_eq!(my_role_ids(&conn, OWNER, "c1").unwrap(), Some(vec![2]));
        delete(&conn, OWNER, "c1").unwrap();
        assert_eq!(my_role_ids(&conn, OWNER, "c1").unwrap(), None);
    }

    #[test]
    fn deleting_a_community_deletes_every_row_that_belongs_to_it() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        fixture::community(&conn, "c2");
        let per_community = [
            "INSERT INTO community_roles (owner_key, community_id, role_id, name) VALUES (?1, ?2, 1, 'r')",
            "INSERT INTO community_members (owner_key, community_id, pseudonym_key, joined_at) VALUES (?1, ?2, 'p', 0)",
            "INSERT INTO event_rsvps (owner_key, community_id, event_id, pseudonym_key, status) VALUES (?1, ?2, 'e', 'p', 'going')",
            "INSERT INTO channel_read_state (owner_key, community_id, channel_id, updated_at) VALUES (?1, ?2, 'ch', 0)",
            "INSERT INTO governance_entries_cache (owner_key, community_id, entries_json, updated_at) VALUES (?1, ?2, '[]', 0)",
            "INSERT INTO message_delivery (owner_key, message_id, community_id, recipient_pseudonym) VALUES (?1, ?2 || '-m', ?2, 'p')",
        ];
        for sql in per_community {
            for id in ["c1", "c2"] {
                conn.execute(sql, params![OWNER, id]).unwrap();
            }
        }
        delete(&conn, OWNER, "c1").unwrap();
        for table in [
            "community_roles",
            "community_members",
            "event_rsvps",
            "channel_read_state",
            "governance_entries_cache",
            "message_delivery",
        ] {
            let left: Vec<String> = conn
                .prepare(&format!("SELECT community_id FROM {table}"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(left, ["c2"], "{table}");
        }
    }

    #[test]
    fn a_row_for_a_community_we_do_not_hold_is_refused() {
        let conn = fixture::conn();
        let err = conn.execute(
            "INSERT INTO community_roles (owner_key, community_id, role_id, name) VALUES (?1, 'gone', 1, 'r')",
            params![OWNER],
        );
        assert!(err.is_err());
    }

    #[test]
    fn an_rsvp_status_outside_the_vocabulary_is_refused() {
        let conn = fixture::conn();
        fixture::community(&conn, "c1");
        let err = conn.execute(
            "INSERT INTO event_rsvps (owner_key, community_id, event_id, pseudonym_key, status) \
             VALUES (?1, 'c1', 'e', 'p', 'none')",
            params![OWNER],
        );
        assert!(err.is_err());
    }
}
