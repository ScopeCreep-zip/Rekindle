//! `community_members`: what we know of each member of each community,
//! per local identity. Every query is scoped to the owner: two identities
//! on one device never read each other's rows.

#[cfg(test)]
mod tests;

use crate::row::{query_list, FromRow};
use rusqlite::{params, Connection, OptionalExtension as _};

use rekindle_types::member::MemberInfo;

/// A member's stored profile, as login loads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRow {
    pub community_id: String,
    pub pseudonym_key: String,
    /// The member's verified card (`M`/0): written only by [`set_card`].
    pub display_name: Option<String>,
    pub custom_status: Option<String>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

impl FromRow for ProfileRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ProfileRow {
            community_id: row.get(0)?,
            pseudonym_key: row.get(1)?,
            display_name: row.get(2)?,
            custom_status: row.get(3)?,
            bio: row.get(4)?,
            pronouns: row.get(5)?,
            theme_color: row.get(6)?,
            badges: crate::row::get_json(row, "badges")?,
            avatar_ref: row.get(8)?,
            banner_ref: row.get(9)?,
        })
    }
}

/// Every stored member profile of `owner_key`.
///
/// # Errors
/// The query failed.
pub fn load_profiles(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<ProfileRow>> {
    query_list(
        conn,
        "SELECT community_id, pseudonym_key, display_name, custom_status, bio, pronouns, \
         theme_color, badges, avatar_ref, banner_ref FROM community_members WHERE owner_key = ?1",
        params![owner_key],
    )
}

/// A member as the member list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterRow {
    pub pseudonym_key: String,
    pub display_name: Option<String>,
}

impl FromRow for RosterRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(RosterRow {
            pseudonym_key: row.get(0)?,
            display_name: row.get(1)?,
        })
    }
}

/// The community's members, by display name.
///
/// # Errors
/// The query failed.
pub fn roster(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<RosterRow>> {
    query_list(
        conn,
        "SELECT pseudonym_key, display_name FROM community_members \
         WHERE owner_key = ?1 AND community_id = ?2 ORDER BY display_name",
        params![owner_key, community_id],
    )
}

/// Store a member unless already stored: a creator or joiner adding
/// itself, or a peer's join announcement. Unnamed: a member's name is its
/// verified card's, stored by [`set_card`] alone (plan E3.4 S6).
///
/// # Errors
/// The write failed.
pub fn insert_if_absent(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    joined_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO community_members \
         (owner_key, community_id, pseudonym_key, joined_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![owner_key, community_id, pseudonym_key, joined_at],
    )?;
    Ok(())
}

/// Cache what `pseudonym_key`'s verified card (`M`/0, read by
/// `read_card`) says: the only writer of a member's `display_name` and
/// `custom_status` (plan E3.4 S6). Stores the member when not yet stored.
///
/// # Errors
/// The write failed.
pub fn set_card(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    card: (Option<&str>, Option<&str>),
    joined_at: i64,
) -> rusqlite::Result<()> {
    let (display_name, custom_status) = card;
    conn.execute(
        "INSERT INTO community_members \
         (owner_key, community_id, pseudonym_key, display_name, custom_status, joined_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(owner_key, community_id, pseudonym_key) DO UPDATE SET \
           display_name = excluded.display_name, custom_status = excluded.custom_status",
        params![
            owner_key,
            community_id,
            pseudonym_key,
            display_name,
            custom_status,
            joined_at
        ],
    )?;
    Ok(())
}

/// Our own profile in one community, as the user set it (plan E3.4 S6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnProfile {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// Store our own profile on our row (`pseudonym_key` is ours), so a
/// restart keeps it.
///
/// # Errors
/// The write failed.
pub fn set_own_profile(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    profile: &OwnProfile,
) -> rusqlite::Result<()> {
    let badges = serde_json::to_string(&profile.badges)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "UPDATE community_members SET bio = ?4, pronouns = ?5, theme_color = ?6, badges = ?7, \
         avatar_ref = ?8, banner_ref = ?9 \
         WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
        params![
            owner_key,
            community_id,
            pseudonym_key,
            profile.bio,
            profile.pronouns,
            profile.theme_color,
            badges,
            profile.avatar_ref,
            profile.banner_ref,
        ],
    )?;
    Ok(())
}

/// Store a member from a bootstrap peer's member list unless already
/// stored; never its peer-supplied name (plan E3.4 S6).
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
         (owner_key, community_id, pseudonym_key, joined_at, \
          bio, pronouns, theme_color, badges) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            owner_key,
            community_id,
            member.pseudonym_key,
            joined_at,
            member.bio,
            member.pronouns,
            member.theme_color,
            badges,
        ],
    )?;
    Ok(())
}

/// A member found in the registry scan. `rekindle-presence` builds these
/// from the signed presence rows it reads. No name: a member's name is its
/// verified card's, never a presence row's (plan E3.4 S6).
#[derive(Debug, Clone)]
pub struct DiscoveredMemberRow {
    pub pseudonym_key: String,
    pub subkey_index: u32,
    pub segment_index: u32,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
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
    let badges = serde_json::to_string(&member.badges)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT INTO community_members \
         (owner_key, community_id, pseudonym_key, joined_at, \
          subkey_index, segment_index, bio, pronouns, theme_color, badges, avatar_ref, banner_ref) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
         ON CONFLICT(owner_key, community_id, pseudonym_key) DO UPDATE SET \
           subkey_index = excluded.subkey_index, segment_index = excluded.segment_index, \
           bio = excluded.bio, pronouns = excluded.pronouns, theme_color = excluded.theme_color, \
           badges = excluded.badges, avatar_ref = excluded.avatar_ref, banner_ref = excluded.banner_ref",
        params![
            owner_key,
            community_id,
            member.pseudonym_key,
            joined_at,
            member.subkey_index,
            member.segment_index,
            member.bio,
            member.pronouns,
            member.theme_color,
            badges,
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

/// Record a member's departure (a leave, kick or ban) at `left_at` (unix
/// ms) for the community's analytics (`rekindle_analytics` member and
/// growth metrics).
///
/// # Errors
/// The write failed.
pub fn record_leave(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    left_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO community_member_leaves \
         (owner_key, community_id, pseudonym_key, left_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![owner_key, community_id, pseudonym_key, left_at],
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
    query_list(
        conn,
        "SELECT pseudonym_key, display_name FROM community_members \
         WHERE owner_key = ?1 AND community_id = ?2 AND display_name IS NOT NULL",
        params![owner_key, community_id],
    )
}
