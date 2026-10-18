use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, SetOutcome};

use crate::error::ProtocolError;

// Layout aliased from `rekindle_types::dht_layout::mailbox` — the
// daemon track indexes the same record and had its own copy.
pub use rekindle_types::dht_layout::mailbox::{
    ROUTE_BLOB as MAILBOX_SUBKEY_ROUTE_BLOB, SUBKEY_COUNT as MAILBOX_SUBKEY_COUNT,
};

/// Create our mailbox record, owned by the identity keypair, and hold it
/// for the session. Returns the lease and the record key.
///
/// The key is random per create (V4: every create adds a random encryption
/// key), so the mailbox is created once, when the identity has none, and
/// re-opened on every later login.
///
/// # Errors
/// The record could not be created.
pub async fn create_mailbox(
    pool: &dyn RecordPort,
    identity_keypair: &str,
) -> Result<(LeaseId, String), ProtocolError> {
    let (lease, key, _) = pool
        .create(&rekindle_records::schema::mailbox(), Some(identity_keypair))
        .await?;
    tracing::info!(key = %key, "created mailbox DHT record");
    Ok((lease, key))
}

/// Hold our existing mailbox writable for the session.
///
/// # Errors
/// The record could not be opened within the pool's retry budget. That is
/// an error for login to report: re-creating would change the key peers
/// know.
pub async fn open_mailbox_writable(
    pool: &dyn RecordPort,
    key: &str,
    identity_keypair: &str,
) -> Result<LeaseId, ProtocolError> {
    let lease = pool.acquire(key, Some(identity_keypair)).await?;
    tracing::debug!(key, "opened mailbox DHT record (writable)");
    Ok(lease)
}

/// A peer's current route blob from their mailbox, or `None` if they have
/// not published one.
///
/// # Errors
/// The mailbox could not be opened or read.
pub async fn read_peer_mailbox_route(
    pool: &dyn RecordPort,
    mailbox_key: &str,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    Ok(pool
        .read_once(mailbox_key, MAILBOX_SUBKEY_ROUTE_BLOB, true)
        .await?
        .map(|v| v.data))
}

/// Publish our current route blob in our mailbox, after each route
/// allocation, so peers can find us after we were offline. Needs the
/// session's writable lease (taken at login by [`open_mailbox_writable`]
/// or [`create_mailbox`]).
///
/// # Errors
/// The record could not be reached, or the write failed outright.
pub async fn update_mailbox_route(
    pool: &dyn RecordPort,
    mailbox_key: &str,
    route_blob: &[u8],
) -> Result<SetOutcome, ProtocolError> {
    let lease = pool.acquire(mailbox_key, None).await?;
    let outcome = pool
        .set_durable(lease, MAILBOX_SUBKEY_ROUTE_BLOB, route_blob.to_vec(), None)
        .await;
    pool.release(lease).await;
    let outcome = outcome?;
    tracing::debug!(key = %mailbox_key, ?outcome, "updated mailbox route blob");
    Ok(outcome)
}
