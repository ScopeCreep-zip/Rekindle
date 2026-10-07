use rekindle_records::lease::LeaseId;
use veilid_core::{DHTSchema, KeyPair};

use super::parse_record_key;
use super::pool::{RecordPool, SetOutcome};
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
    pool: &RecordPool,
    identity_keypair: KeyPair,
) -> Result<(LeaseId, String), ProtocolError> {
    let schema = DHTSchema::dflt(MAILBOX_SUBKEY_COUNT)
        .map_err(|e| ProtocolError::DhtError(format!("invalid mailbox schema: {e}")))?;
    let (lease, key, _) = pool.create(schema, Some(identity_keypair)).await?;
    let key_string = key.to_string();
    tracing::info!(key = %key_string, "created mailbox DHT record");
    Ok((lease, key_string))
}

/// Hold our existing mailbox writable for the session.
///
/// # Errors
/// The record could not be opened within the pool's retry budget. That is
/// an error for login to report: re-creating would change the key peers
/// know.
pub async fn open_mailbox_writable(
    pool: &RecordPool,
    key: &str,
    identity_keypair: KeyPair,
) -> Result<LeaseId, ProtocolError> {
    let lease = pool
        .acquire(&parse_record_key(key)?, Some(identity_keypair))
        .await?;
    tracing::debug!(key, "opened mailbox DHT record (writable)");
    Ok(lease)
}

/// A peer's current route blob from their mailbox, or `None` if they have
/// not published one.
///
/// # Errors
/// The mailbox could not be opened or read.
pub async fn read_peer_mailbox_route(
    pool: &RecordPool,
    mailbox_key: &str,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    let lease = pool.acquire(&parse_record_key(mailbox_key)?, None).await?;
    let value = pool.get(lease, MAILBOX_SUBKEY_ROUTE_BLOB, true).await;
    pool.release(lease).await;
    Ok(value?.map(|v| v.data().to_vec()))
}

/// Publish our current route blob in our mailbox, after each route
/// allocation, so peers can find us after we were offline. Needs the
/// session's writable lease (taken at login by [`open_mailbox_writable`]
/// or [`create_mailbox`]).
///
/// # Errors
/// The record could not be reached, or the write failed outright.
pub async fn update_mailbox_route(
    pool: &RecordPool,
    mailbox_key: &str,
    route_blob: &[u8],
) -> Result<SetOutcome, ProtocolError> {
    let lease = pool.acquire(&parse_record_key(mailbox_key)?, None).await?;
    let outcome = pool
        .set_durable(lease, MAILBOX_SUBKEY_ROUTE_BLOB, route_blob.to_vec())
        .await;
    pool.release(lease).await;
    let outcome = outcome?;
    tracing::debug!(key = %mailbox_key, ?outcome, "updated mailbox route blob");
    Ok(outcome)
}
