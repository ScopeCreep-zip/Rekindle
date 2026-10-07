//! `community_members`: what we know of each member of each community,
//! per local identity. Every query is scoped to the owner: two identities
//! on one device never read each other's rows.

mod roles;
#[cfg(test)]
mod tests;

pub use roles::{add_role, remove_role, remove_role_everywhere, role_ids, set_role_ids};

use rusqlite::{params, Connection, OptionalExtension as _};

use rekindle_types::member::MemberInfo;

use super::role_ids;

/// A unix-seconds value as SQLite stores it (`INTEGER` is `i64`).
fn seconds(value: Option<u64>) -> rusqlite::Result<Option<i64>> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

/// A member's stored profile, as login loads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRow {
    pub community_id: String,
    pub pseudonym_key: String,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<i64>,
    pub badges_json: String,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// Every stored member profile of `owner_key`.
///
/// # Errors
/// The query failed.
pub fn load_profiles(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<ProfileRow>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, pseudonym_key, display_name, bio, pronouns, theme_color, badges, \
         avatar_ref, banner_ref FROM community_members WHERE owner_key = ?1",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok(ProfileRow {
                community_id: row.get(0)?,
                pseudonym_key: row.get(1)?,
                display_name: row.get(2)?,
                bio: row.get(3)?,
                pronouns: row.get(4)?,
                theme_color: row.get(5)?,
                badges_json: row.get(6)?,
                avatar_ref: row.get(7)?,
                banner_ref: row.get(8)?,
            })
        })?
        .collect();
    rows
}

/// A member as the member list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterRow {
    pub pseudonym_key: String,
    pub display_name: Option<String>,
    pub role_ids: Vec<u32>,
    pub timeout_until: Option<i64>,
}

/// The community's members, by display name.
///
/// # Errors
/// The query failed or a role-id set does not decode.
pub fn roster(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<RosterRow>> {
    let mut stmt = conn.prepare(
        "SELECT pseudonym_key, display_name, role_ids, timeout_until FROM community_members \
         WHERE owner_key = ?1 AND community_id = ?2 ORDER BY display_name",
    )?;
    let rows = stmt
        .query_map(params![owner_key, community_id], |row| {
            Ok(RosterRow {
                pseudonym_key: row.get(0)?,
                display_name: row.get(1)?,
                role_ids: role_ids::decode(2, &row.get::<_, String>(2)?)?,
                timeout_until: row.get(3)?,
            })
        })?
        .collect();
    rows
}

/// Store a member unless already stored: a creator or joiner adding
/// itself, or a peer's join announcement.
///
/// # Errors
/// The write failed.
pub fn insert_if_absent(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    role_ids: &[u32],
    joined_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO community_members \
         (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            owner_key,
            community_id,
            pseudonym_key,
            display_name,
            role_ids::encode(role_ids)?,
            joined_at
        ],
    )?;
    Ok(())
}

/// Store a member from a bootstrap peer's member list unless already
/// stored.
///
/// # Errors
/// The write failed.
pub fn insert_bootstrap_if_absent(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    member: &MemberInfo,
    joined_at: i64,
) -> rusqlite::Result<()> {
    let badges = serde_json::to_string(&member.badges)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT OR IGNORE INTO community_members \
         (owner_key, community_id, pseudonym_key, display_name, role_ids, timeout_until, \
          joined_at, bio, pronouns, theme_color, badges) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            owner_key,
            community_id,
            member.pseudonym_key,
            member.display_name,
            role_ids::encode(&member.role_ids)?,
            seconds(member.timeout_until)?,
            joined_at,
            member.bio,
            member.pronouns,
            member.theme_color,
            badges,
        ],
    )?;
    Ok(())
}

/// A member from a `JoinAccepted` member list.
#[derive(Debug, Clone, Copy)]
pub struct AcceptedMember<'a> {
    pub pseudonym_key: &'a str,
    pub display_name: &'a str,
    pub role_ids: &'a [u32],
    pub joined_at: i64,
    pub subkey_index: u32,
    pub onboarding_complete: bool,
    pub timeout_until: Option<i64>,
}

/// Store a `JoinAccepted` member, replacing what was stored.
///
/// # Errors
/// The write failed.
pub fn replace_accepted(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    member: &AcceptedMember<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO community_members \
         (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at, \
          subkey_index, onboarding_complete, timeout_until) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            owner_key,
            community_id,
            member.pseudonym_key,
            member.display_name,
            role_ids::encode(member.role_ids)?,
            member.joined_at,
            member.subkey_index,
            member.onboarding_complete,
            member.timeout_until,
        ],
    )?;
    Ok(())
}

/// A member found in the registry scan, as stored: the role and badge
/// sets JSON-encoded. `rekindle-presence` builds these from the signed
/// presence rows it reads.
#[derive(Debug, Clone)]
pub struct DiscoveredMemberRow {
    pub pseudonym_key: String,
    pub display_name: Option<String>,
    pub role_ids_json: String,
    pub subkey_index: i64,
    pub segment_index: i64,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<i64>,
    pub badges_json: String,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// Store a discovered member, updating everything the scan observes
/// (`joined_at` and onboarding stay as first stored).
///
/// # Errors
/// The write failed.
pub fn upsert_discovered(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    member: &DiscoveredMemberRow,
    joined_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO community_members \
         (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at, \
          subkey_index, segment_index, bio, pronouns, theme_color, badges, avatar_ref, banner_ref) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
         ON CONFLICT(owner_key, community_id, pseudonym_key) DO UPDATE SET \
           display_name = excluded.display_name, role_ids = excluded.role_ids, \
           subkey_index = excluded.subkey_index, segment_index = excluded.segment_index, \
           bio = excluded.bio, pronouns = excluded.pronouns, theme_color = excluded.theme_color, \
           badges = excluded.badges, avatar_ref = excluded.avatar_ref, banner_ref = excluded.banner_ref",
        params![
            owner_key,
            community_id,
            member.pseudonym_key,
            member.display_name,
            member.role_ids_json,
            joined_at,
            member.subkey_index,
            member.segment_index,
            member.bio,
            member.pronouns,
            member.theme_color,
            member.badges_json,
            member.avatar_ref,
            member.banner_ref,
        ],
    )?;
    Ok(())
}

/// Delete the member's row.
///
/// # Errors
/// The write failed.
pub fn delete(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM community_members \
         WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
        params![owner_key, community_id, pseudonym_key],
    )?;
    Ok(())
}

/// Set (unix seconds) or clear (`None`) the member's timeout.
///
/// # Errors
/// The write failed, or `until` is beyond `i64`.
pub fn set_timeout(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    until: Option<u64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE community_members SET timeout_until = ?1 \
         WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
        params![seconds(until)?, owner_key, community_id, pseudonym_key],
    )?;
    Ok(())
}

/// Mark the member's onboarding complete.
///
/// # Errors
/// The write failed.
pub fn set_onboarding_complete(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE community_members SET onboarding_complete = 1 \
         WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
        params![owner_key, community_id, pseudonym_key],
    )?;
    Ok(())
}

/// Mark our own onboarding complete in the community (our pseudonym is
/// the community row's).
///
/// # Errors
/// The write failed.
pub fn set_my_onboarding_complete(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE community_members SET onboarding_complete = 1 \
         WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = \
           (SELECT my_pseudonym_key FROM communities WHERE owner_key = ?1 AND id = ?2)",
        params![owner_key, community_id],
    )?;
    Ok(())
}

/// Every known registry slot in the community: `(pseudonym, subkey)`.
///
/// # Errors
/// The query failed.
pub fn slots(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<(String, u32)>> {
    let mut stmt = conn.prepare(
        "SELECT pseudonym_key, subkey_index FROM community_members \
         WHERE owner_key = ?1 AND community_id = ?2 AND subkey_index IS NOT NULL",
    )?;
    let rows = stmt
        .query_map(params![owner_key, community_id], |row| {
            Ok((
                row.get(0)?,
                u32::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
            ))
        })?
        .collect();
    rows
}

/// The member's registry slot: `(subkey, segment)`, once discovered.
///
/// # Errors
/// The query failed.
pub fn slot(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
) -> rusqlite::Result<Option<(u32, u32)>> {
    Ok(conn
        .query_row(
            "SELECT subkey_index, segment_index FROM community_members \
             WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
            params![owner_key, community_id, pseudonym_key],
            |row| Ok((row.get::<_, Option<u32>>(0)?, row.get::<_, u32>(1)?)),
        )
        .optional()?
        .and_then(|(subkey, segment)| Some((subkey?, segment))))
}

/// The member's display name in the community.
///
/// # Errors
/// The query failed.
pub fn display_name(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
) -> rusqlite::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT display_name FROM community_members \
             WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
            params![owner_key, community_id, pseudonym_key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// The display name the pseudonym has in any of our communities.
///
/// # Errors
/// The query failed.
pub fn any_display_name(
    conn: &Connection,
    owner_key: &str,
    pseudonym_key: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT display_name FROM community_members \
         WHERE owner_key = ?1 AND pseudonym_key = ?2 AND display_name IS NOT NULL LIMIT 1",
        params![owner_key, pseudonym_key],
        |row| row.get(0),
    )
    .optional()
}

/// Every named member of the community: `(pseudonym, display name)`.
///
/// # Errors
/// The query failed.
pub fn display_names(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT pseudonym_key, display_name FROM community_members \
         WHERE owner_key = ?1 AND community_id = ?2 AND display_name IS NOT NULL",
    )?;
    let rows = stmt
        .query_map(params![owner_key, community_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect();
    rows
}
