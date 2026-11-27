//! Local search (plan E7.2 G11, SK2-G design §8): what the user already
//! holds, by the scope the search was opened from. Names match as a
//! case-insensitive substring (`LIKE` with its wildcards escaped); a
//! conversation's messages go through the FTS5 search, limited to that
//! direct conversation. A blank query finds nothing.

use rekindle_types::dto::local_search::{LocalHit, LocalSearchScope};
use rekindle_types::dto::transfer::TransferTarget;
use rekindle_types::search::{MessageSearch, SearchFilters, SearchSort, MAX_SEARCH_LIMIT};
use rusqlite::Connection;

use crate::row::{query_list, FromRow};

/// At most this many hits of each kind.
const LOCAL_HITS_PER_KIND: i64 = 50;

/// `owner_key`'s hits for `query` within `scope`.
///
/// # Errors
/// A query failed, or a stored transfer target is not valid JSON.
pub fn local_search(
    conn: &Connection,
    owner_key: &str,
    scope: &LocalSearchScope,
    query: &str,
) -> rusqlite::Result<Vec<LocalHit>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = like_pattern(query);
    match scope {
        LocalSearchScope::Friends => {
            let mut hits = friends(conn, owner_key, &pattern)?;
            hits.extend(group_dms(conn, owner_key, &pattern)?);
            hits.extend(members(conn, owner_key, &pattern)?);
            Ok(hits)
        }
        LocalSearchScope::Servers => servers(conn, owner_key, &pattern),
        LocalSearchScope::Files => files(conn, owner_key, &pattern),
        LocalSearchScope::Conversation { conversation_id } => {
            let request = MessageSearch {
                query: query.to_owned(),
                filters: SearchFilters {
                    in_conversation: Some(conversation_id.clone()),
                    ..SearchFilters::default()
                },
                sort: SearchSort::Newest,
                limit: MAX_SEARCH_LIMIT,
                offset: 0,
            };
            Ok(super::search(conn, owner_key, &request)?
                .into_iter()
                .map(LocalHit::Message)
                .collect())
        }
    }
}

/// `%query%` with `\`, `%` and `_` escaped (matched with `ESCAPE '\'`).
fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for c in query.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// The rows of one hit kind: `sql` binds `?1` the owner, `?2` the escaped
/// `LIKE` pattern and `?3` the per-kind cap.
fn hits<T: FromRow>(
    conn: &Connection,
    sql: &str,
    owner: &str,
    pattern: &str,
) -> rusqlite::Result<Vec<T>> {
    query_list(
        conn,
        sql,
        rusqlite::params![owner, pattern, LOCAL_HITS_PER_KIND],
    )
}

/// Accepted friends by our nickname, their display name or their key.
fn friends(conn: &Connection, owner: &str, pattern: &str) -> rusqlite::Result<Vec<LocalHit>> {
    let rows: Vec<(String, String)> = hits(
        conn,
        "SELECT f.public_key, COALESCE(n.nickname, f.display_name, '') AS name \
           FROM friends f \
           LEFT JOIN friend_nicknames n \
             ON n.owner_key = f.owner_key AND n.contact_key = f.public_key \
          WHERE f.owner_key = ?1 AND f.friendship_state = 'accepted' \
            AND (n.nickname LIKE ?2 ESCAPE '\\' OR f.display_name LIKE ?2 ESCAPE '\\' \
                 OR f.public_key LIKE ?2 ESCAPE '\\') \
          ORDER BY name COLLATE NOCASE, f.public_key LIMIT ?3",
        owner,
        pattern,
    )?;
    Ok(rows
        .into_iter()
        .map(|(public_key, name)| LocalHit::Friend { public_key, name })
        .collect())
}

/// Our group DMs by name.
fn group_dms(conn: &Connection, owner: &str, pattern: &str) -> rusqlite::Result<Vec<LocalHit>> {
    let rows: Vec<(String, String)> = hits(
        conn,
        "SELECT group_id, name FROM group_dms \
          WHERE owner_key = ?1 AND name LIKE ?2 ESCAPE '\\' \
          ORDER BY name COLLATE NOCASE, group_id LIMIT ?3",
        owner,
        pattern,
    )?;
    Ok(rows
        .into_iter()
        .map(|(group_id, name)| LocalHit::GroupDm { group_id, name })
        .collect())
}

/// Community members by their verified card's name (never a peer-supplied
/// one: `display_name` is written only from the card).
fn members(conn: &Connection, owner: &str, pattern: &str) -> rusqlite::Result<Vec<LocalHit>> {
    let rows: Vec<(String, String, String)> = hits(
        conn,
        "SELECT community_id, pseudonym_key, display_name FROM community_members \
          WHERE owner_key = ?1 AND display_name LIKE ?2 ESCAPE '\\' \
          ORDER BY display_name COLLATE NOCASE, community_id, pseudonym_key LIMIT ?3",
        owner,
        pattern,
    )?;
    Ok(rows
        .into_iter()
        .map(|(community, pseudonym, name)| LocalHit::Member {
            community,
            pseudonym,
            name,
        })
        .collect())
}

/// Favourite servers by label or address, then recent servers (not also
/// favourites) by address, the most recent first.
fn servers(conn: &Connection, owner: &str, pattern: &str) -> rusqlite::Result<Vec<LocalHit>> {
    let rows: Vec<(u32, String, String)> = hits(
        conn,
        "SELECT game_id, address, label, 0 AS recent, added_at AS at FROM favourite_servers \
          WHERE owner_key = ?1 AND (label LIKE ?2 ESCAPE '\\' OR address LIKE ?2 ESCAPE '\\') \
         UNION ALL \
         SELECT r.game_id, r.address, '', 1, r.last_seen FROM recent_servers r \
          WHERE r.owner_key = ?1 AND r.address LIKE ?2 ESCAPE '\\' \
            AND NOT EXISTS (SELECT 1 FROM favourite_servers f WHERE f.owner_key = r.owner_key \
                             AND f.game_id = r.game_id AND f.address = r.address) \
          ORDER BY recent, at DESC LIMIT ?3",
        owner,
        pattern,
    )?;
    Ok(rows
        .into_iter()
        .map(|(game_id, address, label)| LocalHit::Server {
            game_id,
            address,
            label,
        })
        .collect())
}

/// Held attachments (transfers) by the name they save as, newest first.
fn files(conn: &Connection, owner: &str, pattern: &str) -> rusqlite::Result<Vec<LocalHit>> {
    let mut stmt = conn.prepare(
        "SELECT attachment_id, target_json, dest_name FROM transfers \
          WHERE owner_key = ?1 AND dest_name LIKE ?2 ESCAPE '\\' \
          ORDER BY started_at DESC, attachment_id LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![owner, pattern, LOCAL_HITS_PER_KIND],
        |row| {
            let json: String = row.get(1)?;
            let target: TransferTarget = serde_json::from_str(&json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(LocalHit::File {
                attachment_id: row.get(0)?,
                target,
                name: row.get(2)?,
            })
        },
    )?;
    rows.collect()
}
