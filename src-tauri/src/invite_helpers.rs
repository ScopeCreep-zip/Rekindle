use rusqlite::OptionalExtension;

use crate::db::{self, DbPool};
use crate::db_helpers::{db_call, db_call_or_default, db_fire};

/// 48 hours in milliseconds.
const INVITE_EXPIRY_MS: i64 = 48 * 60 * 60 * 1000;

/// Create a tracked outgoing invite in the database.
pub async fn create_outgoing_invite(
    pool: &DbPool,
    owner_key: &str,
    invite_id: &str,
    url: &str,
) -> Result<(), String> {
    let timestamp = db::timestamp_now();
    let expires_at = timestamp + INVITE_EXPIRY_MS;
    let ok = owner_key.to_string();
    let iid = invite_id.to_string();
    let u = url.to_string();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO outgoing_invites (owner_key, invite_id, url, created_at, expires_at, status) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
            rusqlite::params![ok, iid, u, timestamp, expires_at],
        )?;
        Ok(())
    })
    .await
}

/// Cancel a pending outgoing invite. Returns Ok even if no row matched.
pub async fn cancel_outgoing_invite(
    pool: &DbPool,
    owner_key: &str,
    invite_id: &str,
) -> Result<(), String> {
    let ok = owner_key.to_string();
    let iid = invite_id.to_string();
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE outgoing_invites SET status='cancelled' \
             WHERE owner_key=?1 AND invite_id=?2 AND status='pending'",
            rusqlite::params![ok, iid],
        )?;
        Ok(())
    })
    .await
}

/// Check if an `invite_id` is cancelled (for auto-rejecting incoming requests).
pub async fn is_invite_cancelled(pool: &DbPool, owner_key: &str, invite_id: &str) -> bool {
    let ok = owner_key.to_string();
    let iid = invite_id.to_string();
    db_call_or_default(pool, move |conn| {
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM outgoing_invites WHERE owner_key=?1 AND invite_id=?2",
                rusqlite::params![ok, iid],
                |row| row.get(0),
            )
            .optional()?;
        Ok(status.as_deref() == Some("cancelled"))
    })
    .await
}

/// Mark an invite as 'responded' when someone sends a `FriendRequest` with it.
pub fn mark_invite_responded(pool: &DbPool, owner_key: &str, invite_id: &str, responder_key: &str) {
    let ok = owner_key.to_string();
    let iid = invite_id.to_string();
    let rk = responder_key.to_string();
    db_fire(pool, "mark invite responded", move |conn| {
        conn.execute(
            "UPDATE outgoing_invites SET status='responded', accepted_by=?3 \
             WHERE owner_key=?1 AND invite_id=?2 AND status='pending'",
            rusqlite::params![ok, iid, rk],
        )?;
        Ok(())
    });
}

/// Mark invite as 'accepted' after manual acceptance.
pub fn mark_invite_accepted(pool: &DbPool, owner_key: &str, invite_id: &str) {
    let ok = owner_key.to_string();
    let iid = invite_id.to_string();
    db_fire(pool, "mark invite accepted", move |conn| {
        conn.execute(
            "UPDATE outgoing_invites SET status='accepted' \
             WHERE owner_key=?1 AND invite_id=?2 AND status='responded'",
            rusqlite::params![ok, iid],
        )?;
        Ok(())
    });
}

/// Mark invite as 'rejected' after manual rejection.
pub fn mark_invite_rejected(pool: &DbPool, owner_key: &str, invite_id: &str) {
    let ok = owner_key.to_string();
    let iid = invite_id.to_string();
    db_fire(pool, "mark invite rejected", move |conn| {
        conn.execute(
            "UPDATE outgoing_invites SET status='rejected' \
             WHERE owner_key=?1 AND invite_id=?2",
            rusqlite::params![ok, iid],
        )?;
        Ok(())
    });
}

/// Expire all stale pending invites past their `expires_at` time.
pub fn expire_stale_invites(pool: &DbPool, owner_key: &str) {
    let ok = owner_key.to_string();
    let now = db::timestamp_now();
    db_fire(pool, "expire old invites", move |conn| {
        conn.execute(
            "UPDATE outgoing_invites SET status='expired' \
             WHERE owner_key=?1 AND status='pending' AND expires_at < ?2",
            rusqlite::params![ok, now],
        )?;
        Ok(())
    });
}

/// Fetch all pending outgoing invites for display.
pub async fn get_pending_invites(
    pool: &DbPool,
    owner_key: &str,
) -> Result<Vec<OutgoingInvite>, String> {
    let ok = owner_key.to_string();
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT invite_id, url, created_at, expires_at, status, accepted_by \
             FROM outgoing_invites WHERE owner_key=?1 AND status IN ('pending', 'responded') \
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![ok], |row| {
            Ok(OutgoingInvite {
                invite_id: row.get(0)?,
                url: row.get(1)?,
                created_at: row.get(2)?,
                expires_at: row.get(3)?,
                status: row.get(4)?,
                accepted_by: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}

/// Outgoing invite data for IPC.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingInvite {
    pub invite_id: String,
    pub url: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub status: String,
    pub accepted_by: Option<String>,
}
