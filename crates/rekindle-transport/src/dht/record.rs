//! Generic DHT record operations — the Veilid boundary for all record I/O.
//!
//! All DHT read/write/watch/inspect calls in the workspace go through
//! the functions in this module. No other code touches `RoutingContext`
//! DHT methods directly.

use veilid_core::{
    DHTSchema, DHTSchemaSMPLMember, KeyPair, RoutingContext, SetDHTValueOptions,
    ValueSubkeyRangeSet, CRYPTO_KIND_VLD0,
};

use crate::error::{TransportError, Result};

/// Parse a DHT record key string into a Veilid `RecordKey`.
pub fn parse_key(key: &str) -> Result<veilid_core::RecordKey> {
    key.parse().map_err(|e| TransportError::DhtError {
        reason: format!("invalid record key '{key}': {e}"),
    })
}

/// Create a DFLT record (single owner, N subkeys).
///
/// Returns `(key_string, owner_keypair)`. The keypair MUST be persisted.
pub async fn create_dflt(
    rc: &RoutingContext,
    subkey_count: u16,
    owner: Option<KeyPair>,
) -> Result<(String, Option<KeyPair>)> {
    let schema = DHTSchema::dflt(subkey_count)
        .map_err(|e| TransportError::RecordCreateFailed { reason: format!("schema: {e}") })?;

    let desc = rc
        .create_dht_record(CRYPTO_KIND_VLD0, schema, owner)
        .await
        .map_err(|e| TransportError::RecordCreateFailed { reason: format!("{e}") })?;

    let key = desc.key().to_string();
    let keypair = desc.owner_secret().map(|s| {
        KeyPair::new_from_parts(desc.owner().clone(), s.value())
    });

    Ok((key, keypair))
}

/// Create a SMPL record (multi-writer with member slots).
///
/// Returns `(key_string, owner_keypair)`.
pub async fn create_smpl(
    rc: &RoutingContext,
    owner_subkey_count: u16,
    members: Vec<DHTSchemaSMPLMember>,
) -> Result<(String, Option<KeyPair>)> {
    let schema = DHTSchema::smpl(owner_subkey_count, members)
        .map_err(|e| TransportError::RecordCreateFailed { reason: format!("SMPL schema: {e}") })?;

    let desc = rc
        .create_dht_record(CRYPTO_KIND_VLD0, schema, None)
        .await
        .map_err(|e| TransportError::RecordCreateFailed { reason: format!("{e}") })?;

    let key = desc.key().to_string();
    let keypair = desc.owner_secret().map(|s| {
        KeyPair::new_from_parts(desc.owner().clone(), s.value())
    });

    Ok((key, keypair))
}

/// Open an existing record for reading (no writer).
pub async fn open_readonly(rc: &RoutingContext, key: &str) -> Result<()> {
    let rk = parse_key(key)?;
    let _ = rc.open_dht_record(rk, None)
        .await
        .map_err(|e| TransportError::DhtError { reason: format!("open readonly: {e}") })?;
    Ok(())
}

/// Open an existing record with write access.
pub async fn open_writable(rc: &RoutingContext, key: &str, writer: KeyPair) -> Result<()> {
    let rk = parse_key(key)?;
    let _ = rc.open_dht_record(rk, Some(writer))
        .await
        .map_err(|e| TransportError::DhtError { reason: format!("open writable: {e}") })?;
    Ok(())
}

/// Close a record.
pub async fn close(rc: &RoutingContext, key: &str) -> Result<()> {
    let rk = parse_key(key)?;
    rc.close_dht_record(rk)
        .await
        .map_err(|e| TransportError::DhtError { reason: format!("close: {e}") })?;
    Ok(())
}

/// Read a subkey value. Returns `None` if not yet set.
pub async fn get(
    rc: &RoutingContext,
    key: &str,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>> {
    let rk = parse_key(key)?;
    let value = rc
        .get_dht_value(rk, subkey, force_refresh)
        .await
        .map_err(|e| TransportError::DhtError { reason: format!("get: {e}") })?;
    Ok(value.map(|v| v.data().to_vec()))
}

/// Write a subkey value. Optionally specify an explicit writer keypair.
pub async fn set(
    rc: &RoutingContext,
    key: &str,
    subkey: u32,
    data: Vec<u8>,
    writer: Option<KeyPair>,
) -> Result<()> {
    let rk = parse_key(key)?;

    if data.len() > 32_768 {
        return Err(TransportError::SubkeyTooLarge {
            subkey,
            size: data.len(),
            max: 32_768,
        });
    }

    let options = writer.map(|w| SetDHTValueOptions {
        writer: Some(w),
        ..Default::default()
    });

    rc.set_dht_value(rk, subkey, data, options)
        .await
        .map_err(|e| TransportError::DhtError { reason: format!("set: {e}") })?;

    Ok(())
}

/// Watch specific subkeys for changes.
///
/// Returns `true` if the watch is active, `false` if cancelled/failed.
pub async fn watch(rc: &RoutingContext, key: &str, subkeys: &[u32]) -> Result<bool> {
    let rk = parse_key(key)?;
    let range: ValueSubkeyRangeSet = subkeys.iter().copied().collect();

    rc.watch_dht_values(rk, Some(range), None, None)
        .await
        .map_err(|e| TransportError::DhtError { reason: format!("watch: {e}") })
}

/// Inspect a record to get sequence numbers without fetching data.
///
/// Returns a vec of `(subkey, local_seq, network_seq)` for changed subkeys.
pub async fn inspect(
    rc: &RoutingContext,
    key: &str,
    subkeys: Option<&[u32]>,
) -> Result<veilid_core::DHTRecordReport> {
    let rk = parse_key(key)?;
    let range = subkeys.map(|s| s.iter().copied().collect::<ValueSubkeyRangeSet>());

    rc.inspect_dht_record(rk, range, veilid_core::DHTReportScope::SyncGet)
        .await
        .map_err(|e| TransportError::DhtError { reason: format!("inspect: {e}") })
}

/// Try to open an existing record writable, falling back to creating a new one.
///
/// Returns `(key, keypair, is_new)`.
pub async fn open_or_create(
    rc: &RoutingContext,
    existing_key: Option<&str>,
    existing_keypair: Option<KeyPair>,
    subkey_count: u16,
) -> Result<(String, Option<KeyPair>, bool)> {
    if let (Some(key), Some(kp)) = (existing_key, existing_keypair) {
        match open_writable(rc, key, kp.clone()).await {
            Ok(()) => return Ok((key.to_string(), Some(kp), false)),
            Err(e) => {
                tracing::warn!(key, error = %e, "failed to reopen DHT record, creating new");
            }
        }
    }

    let (key, keypair) = create_dflt(rc, subkey_count, None).await?;
    Ok((key, keypair, true))
}
