//! `message_send_log`: the user-content ratchet messages we sent (plan E2.5,
//! D24 send half), so a peer that could not open one can ask for it again.
//!
//! A row is written in the ratchet's transaction (`crate::ratchet::
//! encrypt_record_and_log_outbound`), keyed by the message's header id
//! (`rekindle_crypto::replay::ratchet_message_id`) and holding the header's
//! sending ratchet key. A `RetryRequest` resends a row only when the key the
//! peer observed is the logged one ([`take_for_resend`]); the row is then
//! archived, so a copy of the request resends nothing. The plaintext is at
//! rest under the database's SQLCipher key (D1/D3), and rows older than
//! [`SEND_LOG_MAX_AGE`] are pruned ([`prune`]). This is Signal-Android's
//! `MessageSendLogTables` with its retention tied to the delivery horizon.

use std::time::Duration;

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rusqlite::{params, Connection, OptionalExtension};

use super::identity_text;

/// How long a sent message stays resendable: [`DELIVERY_HORIZON`], past
/// which the peer could not have been waiting for it.
pub const SEND_LOG_MAX_AGE: Duration = DELIVERY_HORIZON;

/// One sent message.
#[derive(Clone, PartialEq, Eq)]
pub struct LoggedSend {
    pub peer: [u8; 32],
    pub msg_id: [u8; 32],
    /// The `DmFrame` kind it was sent as.
    pub type_id: u16,
    /// The header's sending ratchet key.
    pub ratchet_key: [u8; 32],
    /// The frame plaintext (the encoded `DmContent`).
    pub plaintext: Vec<u8>,
    /// Milliseconds.
    pub sent_at: i64,
    /// The content's client id (plan E7.1: its expiry deletes this row);
    /// `None` for a kind with none.
    pub client_id: Option<String>,
}

/// The plaintext is never printed.
impl std::fmt::Debug for LoggedSend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoggedSend")
            .field("type_id", &self.type_id)
            .field("sent_at", &self.sent_at)
            .finish_non_exhaustive()
    }
}

/// Log a sent message (inside the ratchet's transaction).
///
/// # Errors
/// The write failed.
pub fn insert(conn: &Connection, owner: &[u8; 32], sent: &LoggedSend) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO message_send_log (owner_key, peer, msg_id, type_id, ratchet_key, \
         plaintext_sealed, sent_at, client_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            identity_text(owner),
            identity_text(&sent.peer),
            sent.msg_id,
            sent.type_id,
            sent.ratchet_key,
            sent.plaintext,
            sent.sent_at,
            sent.client_id
        ],
    )?;
    Ok(())
}

/// The unarchived row of `msg_id` to `peer`.
///
/// # Errors
/// The query failed.
pub fn get(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
    msg_id: &[u8; 32],
) -> rusqlite::Result<Option<LoggedSend>> {
    conn.query_row(
        "SELECT type_id, ratchet_key, plaintext_sealed, sent_at, client_id FROM message_send_log \
         WHERE owner_key = ?1 AND peer = ?2 AND msg_id = ?3 AND archived_at IS NULL",
        params![identity_text(owner), identity_text(peer), msg_id],
        |row| {
            Ok(LoggedSend {
                peer: *peer,
                msg_id: *msg_id,
                type_id: row.get(0)?,
                ratchet_key: row.get(1)?,
                plaintext: row.get(2)?,
                sent_at: row.get(3)?,
                client_id: row.get(4)?,
            })
        },
    )
    .optional()
}

/// A `RetryRequest` from `peer` for `msg_id`, observed under
/// `observed_ratchet_key`: the row to resend, archived in the same
/// transaction, only when the observed key is the logged one. A request
/// naming another key (forged, or for a message we never sent under it)
/// changes nothing and returns `None`, as does an unknown or archived id.
///
/// # Errors
/// The query failed.
pub fn take_for_resend(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
    msg_id: &[u8; 32],
    observed_ratchet_key: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<Option<LoggedSend>> {
    let tx = conn.unchecked_transaction()?;
    let Some(sent) = get(&tx, owner, peer, msg_id)? else {
        return Ok(None);
    };
    if sent.ratchet_key != *observed_ratchet_key {
        return Ok(None);
    }
    tx.execute(
        "UPDATE message_send_log SET archived_at = ?4 \
         WHERE owner_key = ?1 AND peer = ?2 AND msg_id = ?3",
        params![identity_text(owner), identity_text(peer), msg_id, now_ms],
    )?;
    tx.commit()?;
    Ok(Some(sent))
}

/// Drop the rows sent more than [`SEND_LOG_MAX_AGE`] before `now_ms`; how
/// many went.
///
/// # Errors
/// The write failed.
pub fn prune(conn: &Connection, owner: &[u8; 32], now_ms: i64) -> rusqlite::Result<usize> {
    let max_age_ms = i64::try_from(SEND_LOG_MAX_AGE.as_millis()).expect("days fit i64 ms");
    conn.execute(
        "DELETE FROM message_send_log WHERE owner_key = ?1 AND sent_at < ?2",
        params![identity_text(owner), now_ms.saturating_sub(max_age_ms)],
    )
}

#[cfg(test)]
#[path = "send_log_tests.rs"]
mod tests;
