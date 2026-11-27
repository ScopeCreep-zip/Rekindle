//! `pending_friend_requests`: incoming requests not yet accepted or
//! rejected, one per (owner, requester).

use crate::row::{query_list, query_opt, FromRow};
use rusqlite::{params, Connection, OptionalExtension as _};
use serde::{Deserialize, Serialize};

/// A request as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFriendRequest {
    pub public_key: String,
    pub display_name: String,
    pub message: String,
    pub received_at: i64,
}

impl FromRow for PendingFriendRequest {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(PendingFriendRequest {
            public_key: row.get(0)?,
            display_name: row.get(1)?,
            message: row.get(2)?,
            received_at: row.get(3)?,
        })
    }
}

/// What answering a request needs: where the requester lives and the
/// invite it arrived with (no bundle: the accepter reads the requester's
/// prekey record, plan E2.4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    /// The name the request carried: the Host's own record of who asked
    /// (plan audit U-7), `None` when there is no request.
    pub display_name: Option<String>,
    pub profile_dht_key: Option<String>,
    pub mailbox_dht_key: Option<String>,
    pub route_blob: Option<Vec<u8>>,
    pub invite_id: Option<String>,
    /// The reply inbox the request carried and its one-time writer (plan
    /// RV2): our first contact is written there too.
    pub reply: Option<(String, String)>,
    /// The requester's dm chain record (decision 67 item 2), adopted once
    /// the request is accepted.
    pub chain_record_key: Option<String>,
}

impl FromRow for Answer {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Answer {
            display_name: Some(row.get(0)?),
            profile_dht_key: row.get(1)?,
            mailbox_dht_key: row.get(2)?,
            route_blob: row.get(3)?,
            invite_id: row.get(4)?,
            reply: Some((row.get(5)?, row.get(6)?)),
            chain_record_key: Some(row.get(7)?),
        })
    }
}

/// A request as it arrives.
#[derive(Debug, Clone, Copy)]
pub struct Incoming<'a> {
    pub public_key: &'a str,
    pub display_name: &'a str,
    pub message: &'a str,
    pub received_at: i64,
    pub profile_dht_key: &'a str,
    pub route_blob: &'a [u8],
    pub mailbox_dht_key: &'a str,
    pub invite_id: Option<&'a str>,
    /// The reply inbox's record key and one-time writer (plan RV2).
    pub reply_inbox_key: &'a str,
    pub reply_writer: &'a str,
    /// The requester's dm chain record (decision 67 item 2).
    pub chain_record_key: &'a str,
}

/// Store a request, replacing an earlier, different one from the same
/// requester. Whether it changed anything: the same request again (every
/// field but `received_at` equal) is left as it was, `false`.
///
/// # Errors
/// The write failed.
pub fn upsert(
    conn: &Connection,
    owner_key: &str,
    request: &Incoming<'_>,
) -> rusqlite::Result<bool> {
    let held = conn
        .query_row(
            "SELECT 1 FROM pending_friend_requests \
             WHERE owner_key = ?1 AND public_key = ?2 AND display_name = ?3 AND message = ?4 \
               AND profile_dht_key IS ?5 AND route_blob IS ?6 AND mailbox_dht_key IS ?7 \
               AND invite_id IS ?8 AND reply_inbox_key = ?9 AND reply_writer = ?10 \
               AND chain_record_key = ?11",
            params![
                owner_key,
                request.public_key,
                request.display_name,
                request.message,
                request.profile_dht_key,
                request.route_blob,
                request.mailbox_dht_key,
                request.invite_id,
                request.reply_inbox_key,
                request.reply_writer,
                request.chain_record_key,
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if held {
        return Ok(false);
    }
    conn.execute(
        "INSERT OR REPLACE INTO pending_friend_requests \
         (owner_key, public_key, display_name, message, received_at, profile_dht_key, \
          route_blob, mailbox_dht_key, invite_id, reply_inbox_key, reply_writer, \
          chain_record_key) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            owner_key,
            request.public_key,
            request.display_name,
            request.message,
            request.received_at,
            request.profile_dht_key,
            request.route_blob,
            request.mailbox_dht_key,
            request.invite_id,
            request.reply_inbox_key,
            request.reply_writer,
            request.chain_record_key,
        ],
    )?;
    Ok(true)
}

/// Every request, oldest first.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<PendingFriendRequest>> {
    query_list(
        conn,
        "SELECT public_key, display_name, message, received_at \
         FROM pending_friend_requests WHERE owner_key = ?1 ORDER BY received_at",
        params![owner_key],
    )
}

/// What answering the request from `public_key` needs; `None` when there
/// is no such request (an answer to nothing is refused, live A/B finding 68).
///
/// # Errors
/// The query failed.
pub fn answer(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Option<Answer>> {
    query_opt(
        conn,
        "SELECT display_name, profile_dht_key, mailbox_dht_key, route_blob, invite_id, \
                    reply_inbox_key, reply_writer, chain_record_key \
             FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key],
    )
}

/// Delete the request from `public_key`.
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, owner_key: &str, public_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key],
    )?;
    Ok(())
}

/// Delete the requests received before `cutoff`; how many.
///
/// # Errors
/// The write failed.
pub fn delete_received_before(
    conn: &Connection,
    owner_key: &str,
    cutoff: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND received_at < ?2",
        params![owner_key, cutoff],
    )
}

#[cfg(test)]
#[path = "pending_requests_tests.rs"]
mod tests;
