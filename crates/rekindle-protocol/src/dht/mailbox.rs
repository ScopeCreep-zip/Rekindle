use veilid_core::{DHTSchema, RoutingContext, CRYPTO_KIND_VLD0};

use crate::error::ProtocolError;

/// Subkey index for the route blob in the mailbox DHT record.
pub const MAILBOX_SUBKEY_ROUTE_BLOB: u32 = 0;

/// Total subkey count for a mailbox record.
pub const MAILBOX_SUBKEY_COUNT: u16 = 1;

/// Create the mailbox DHT record using the identity keypair as owner.
///
/// The mailbox record key is deterministic for a given identity keypair because
/// Veilid uses the owner keypair to derive the record key. This means the
/// mailbox key is permanent and can be shared in invite links.
///
/// Returns the record key string.
pub async fn create_mailbox(
    rc: &RoutingContext,
    identity_keypair: veilid_core::KeyPair,
) -> Result<String, ProtocolError> {
    let schema = DHTSchema::dflt(MAILBOX_SUBKEY_COUNT)
        .map_err(|e| ProtocolError::DhtError(format!("invalid mailbox schema: {e}")))?;

    let descriptor = rc
        .create_dht_record(CRYPTO_KIND_VLD0, schema, Some(identity_keypair))
        .await
        .map_err(|e| ProtocolError::DhtError(format!("create_mailbox: {e}")))?;

    let key_string = descriptor.key().to_string();
    tracing::info!(key = %key_string, "created mailbox DHT record");
    Ok(key_string)
}

/// Open an existing mailbox for writing (our own).
///
/// Must be called on each login to regain write access to the mailbox record.
pub async fn open_mailbox_writable(
    rc: &RoutingContext,
    key: &str,
    identity_keypair: veilid_core::KeyPair,
) -> Result<(), ProtocolError> {
    let record_key = key
        .parse()
        .map_err(|e| ProtocolError::DhtError(format!("invalid mailbox key '{key}': {e}")))?;

    let _ = rc
        .open_dht_record(record_key, Some(identity_keypair))
        .await
        .map_err(|e| ProtocolError::DhtError(format!("open_mailbox_writable: {e}")))?;

    tracing::debug!(key, "opened mailbox DHT record (writable)");
    Ok(())
}

/// Open a peer's mailbox for reading their route blob.
///
/// Returns the route blob bytes if subkey 0 has been set, or `None` if empty.
pub async fn read_peer_mailbox_route(
    rc: &RoutingContext,
    mailbox_key: &str,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    let record_key: veilid_core::RecordKey = mailbox_key.parse().map_err(|e| {
        ProtocolError::DhtError(format!("invalid mailbox key '{mailbox_key}': {e}"))
    })?;

    // Open read-only (no writer keypair)
    let _ = rc
        .open_dht_record(record_key.clone(), None)
        .await
        .map_err(|e| ProtocolError::DhtError(format!("open peer mailbox: {e}")))?;

    let value = rc
        .get_dht_value(record_key, MAILBOX_SUBKEY_ROUTE_BLOB, false)
        .await
        .map_err(|e| ProtocolError::DhtError(format!("read peer mailbox route: {e}")))?;

    Ok(value.map(|v| v.data().to_vec()))
}

/// Update our route blob in the mailbox.
///
/// Called after each route allocation/refresh so peers can discover our
/// current route even after we've gone offline and come back.
pub async fn update_mailbox_route(
    rc: &RoutingContext,
    mailbox_key: &str,
    route_blob: &[u8],
) -> Result<(), ProtocolError> {
    let record_key = mailbox_key.parse().map_err(|e| {
        ProtocolError::DhtError(format!("invalid mailbox key '{mailbox_key}': {e}"))
    })?;

    rc.set_dht_value(record_key, MAILBOX_SUBKEY_ROUTE_BLOB, route_blob.to_vec(), None)
        .await
        .map_err(|e| ProtocolError::DhtError(format!("update_mailbox_route: {e}")))?;

    tracing::debug!(key = %mailbox_key, "updated mailbox route blob");
    Ok(())
}
