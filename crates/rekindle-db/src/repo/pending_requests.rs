//! `pending_friend_requests`: incoming requests not yet accepted or
//! rejected, one per (owner, requester).

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
               AND invite_id IS ?8",
            params![
                owner_key,
                request.public_key,
                request.display_name,
                request.message,
                request.profile_dht_key,
                request.route_blob,
                request.mailbox_dht_key,
                request.invite_id,
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
          route_blob, mailbox_dht_key, invite_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
        ],
    )?;
    Ok(true)
}

/// Every request, oldest first.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<PendingFriendRequest>> {
    let mut stmt = conn.prepare(
        "SELECT public_key, display_name, message, received_at \
         FROM pending_friend_requests WHERE owner_key = ?1 ORDER BY received_at",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok(PendingFriendRequest {
                public_key: row.get(0)?,
                display_name: row.get(1)?,
                message: row.get(2)?,
                received_at: row.get(3)?,
            })
        })?
        .collect();
    rows
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
    conn
        .query_row(
            "SELECT display_name, profile_dht_key, mailbox_dht_key, route_blob, invite_id \
             FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            params![owner_key, public_key],
            |row| {
                Ok(Answer {
                    display_name: Some(row.get(0)?),
                    profile_dht_key: row.get(1)?,
                    mailbox_dht_key: row.get(2)?,
                    route_blob: row.get(3)?,
                    invite_id: row.get(4)?,
                })
            },
        )
        .optional()
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
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn incoming(public_key: &str, received_at: i64) -> Incoming<'_> {
        Incoming {
            public_key,
            display_name: "Bob",
            message: "hi",
            received_at,
            profile_dht_key: "profile",
            route_blob: &[1, 2],
            mailbox_dht_key: "mailbox",
            invite_id: Some("invite"),
        }
    }

    #[test]
    fn the_same_request_again_changes_nothing() {
        let conn = fixture::conn();
        assert!(upsert(&conn, OWNER, &incoming("bob", 5)).unwrap());
        assert!(!upsert(&conn, OWNER, &incoming("bob", 9)).unwrap());
        assert_eq!(list(&conn, OWNER).unwrap()[0].received_at, 5);
        let changed = Incoming {
            message: "hello again",
            ..incoming("bob", 9)
        };
        assert!(upsert(&conn, OWNER, &changed).unwrap());
        assert_eq!(list(&conn, OWNER).unwrap().len(), 1);
    }

    #[test]
    fn a_stored_request_lists_and_answers() {
        let conn = fixture::conn();
        upsert(&conn, OWNER, &incoming("bob", 5)).unwrap();
        assert_eq!(
            list(&conn, OWNER).unwrap(),
            [PendingFriendRequest {
                public_key: "bob".into(),
                display_name: "Bob".into(),
                message: "hi".into(),
                received_at: 5,
            }]
        );
        let a = answer(&conn, OWNER, "bob").unwrap().expect("bob asked");
        assert_eq!(a.route_blob.as_deref(), Some(&[1, 2][..]));
        assert_eq!(a.invite_id.as_deref(), Some("invite"));
        assert_eq!(answer(&conn, OWNER, "ghost").unwrap(), None, "no request, no answer");
    }

    #[test]
    fn a_second_request_replaces_the_first_and_old_ones_expire() {
        let conn = fixture::conn();
        upsert(&conn, OWNER, &incoming("bob", 5)).unwrap();
        let second = Incoming {
            message: "hi again",
            ..incoming("bob", 9)
        };
        assert!(upsert(&conn, OWNER, &second).unwrap());
        upsert(&conn, OWNER, &incoming("carol", 1)).unwrap();
        assert_eq!(delete_received_before(&conn, OWNER, 6).unwrap(), 1);
        let left = list(&conn, OWNER).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(
            (left[0].public_key.as_str(), left[0].received_at),
            ("bob", 9)
        );
        delete(&conn, OWNER, "bob").unwrap();
        assert!(list(&conn, OWNER).unwrap().is_empty());
    }
}
