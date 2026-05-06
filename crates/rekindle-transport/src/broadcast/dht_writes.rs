//! Veilid DHT primitives — the sole boundary for all DHT record I/O.
//!
//! Every DHT create/open/close/get/set/watch/inspect in the workspace
//! routes through this module. No other code calls `dht::record::*`
//! or `veilid_core::RoutingContext` DHT methods directly.
//!
//! Every operation is traced at `debug` level for developer observability.
//! Failures are traced at `warn`. Audit-sensitive operations (create, set,
//! close) are traced at `info`.

use tracing::{debug, info, warn};

use super::dht;
use super::dht::channel_log::DhtLog;
use crate::error::Result;
use super::node::TransportNode;

// ── Record lifecycle ───────────────────────────────────────────────────

/// Create a DFLT DHT record (single owner, N subkeys).
pub async fn create_dflt(
    node: &TransportNode, subkey_count: u16, owner: Option<veilid_core::KeyPair>,
) -> Result<(String, Option<veilid_core::KeyPair>)> {
    debug!(subkey_count, has_owner = owner.is_some(), "dht: create_dflt");
    let dht = node.dht()?;
    let result = dht::record::create_dflt(dht.routing_context(), subkey_count, owner).await;
    match &result {
        Ok((key, _)) => info!(key = %key, subkey_count, "dht: record created (DFLT)"),
        Err(e) => warn!(error = %e, subkey_count, "dht: create_dflt failed"),
    }
    result
}

/// Create a SMPL DHT record (multi-writer with member slots).
pub async fn create_smpl(
    node: &TransportNode, owner_subkey_count: u16,
    members: Vec<veilid_core::DHTSchemaSMPLMember>,
) -> Result<(String, Option<veilid_core::KeyPair>)> {
    debug!(owner_subkey_count, member_count = members.len(), "dht: create_smpl");
    let dht = node.dht()?;
    let result = dht::record::create_smpl(dht.routing_context(), owner_subkey_count, members).await;
    match &result {
        Ok((key, _)) => info!(key = %key, owner_subkey_count, "dht: record created (SMPL)"),
        Err(e) => warn!(error = %e, "dht: create_smpl failed"),
    }
    result
}

/// Open a DHT record for reading (no writer).
pub async fn open_readonly(node: &TransportNode, record_key: &str) -> Result<()> {
    debug!(record_key, "dht: open_readonly");
    let dht = node.dht()?;
    let result = dht::record::open_readonly(dht.routing_context(), record_key).await;
    if let Err(ref e) = result {
        warn!(record_key, error = %e, "dht: open_readonly failed");
    }
    result
}

/// Open a DHT record with write access.
pub async fn open_writable(
    node: &TransportNode, record_key: &str, writer: veilid_core::KeyPair,
) -> Result<()> {
    debug!(record_key, "dht: open_writable");
    let dht = node.dht()?;
    let result = dht::record::open_writable(dht.routing_context(), record_key, writer).await;
    if let Err(ref e) = result {
        warn!(record_key, error = %e, "dht: open_writable failed");
    }
    result
}

/// Close a DHT record.
pub async fn close(node: &TransportNode, record_key: &str) -> Result<()> {
    info!(record_key, "dht: close");
    let dht = node.dht()?;
    let result = dht::record::close(dht.routing_context(), record_key).await;
    if let Err(ref e) = result {
        warn!(record_key, error = %e, "dht: close failed");
    }
    result
}

// ── Subkey I/O ─────────────────────────────────────────────────────────

/// Read a subkey value. Returns `None` if not yet set.
pub async fn get(
    node: &TransportNode, record_key: &str, subkey: u32, force_refresh: bool,
) -> Result<Option<Vec<u8>>> {
    debug!(record_key, subkey, force_refresh, "dht: get");
    let dht = node.dht()?;
    let result = dht::record::get(dht.routing_context(), record_key, subkey, force_refresh).await;
    match &result {
        Ok(Some(data)) => debug!(record_key, subkey, bytes = data.len(), "dht: get returned data"),
        Ok(None) => debug!(record_key, subkey, "dht: get returned None"),
        Err(e) => warn!(record_key, subkey, error = %e, "dht: get failed"),
    }
    result
}

/// Write raw bytes to a specific subkey.
pub async fn set(
    node: &TransportNode, record_key: &str, subkey: u32,
    data: Vec<u8>, writer: Option<veilid_core::KeyPair>,
) -> Result<()> {
    let data_len = data.len();
    debug!(record_key, subkey, bytes = data_len, has_writer = writer.is_some(), "dht: set");
    let dht = node.dht()?;
    let result = dht::record::set(dht.routing_context(), record_key, subkey, data, writer).await;
    match &result {
        Ok(()) => info!(record_key, subkey, bytes = data_len, "dht: set complete"),
        Err(e) => warn!(record_key, subkey, bytes = data_len, error = %e, "dht: set failed"),
    }
    result
}

// ── Watch ──────────────────────────────────────────────────────────────

/// Set a DHT watch on specific subkeys of a record.
pub async fn watch(
    node: &TransportNode, record_key: &str, subkeys: &[u32],
) -> Result<bool> {
    debug!(record_key, subkey_count = subkeys.len(), "dht: watch");
    let dht = node.dht()?;
    let result = dht::record::watch(dht.routing_context(), record_key, subkeys).await;
    match &result {
        Ok(true) => info!(record_key, subkey_count = subkeys.len(), "dht: watch active"),
        Ok(false) => warn!(record_key, "dht: watch declined by Veilid"),
        Err(e) => warn!(record_key, error = %e, "dht: watch failed"),
    }
    result
}

// ── Inspect ────────────────────────────────────────────────────────────

/// Inspect a record to get sequence numbers without fetching data.
pub async fn inspect(
    node: &TransportNode, record_key: &str, subkeys: Option<&[u32]>,
) -> Result<veilid_core::DHTRecordReport> {
    debug!(record_key, "dht: inspect");
    let dht = node.dht()?;
    let result = dht::record::inspect(dht.routing_context(), record_key, subkeys).await;
    if let Err(ref e) = result {
        warn!(record_key, error = %e, "dht: inspect failed");
    }
    result
}

// ── DhtLog (append-only log built on DHT records) ──────────────────────

/// Create a new DhtLog. Returns `(DhtLog, owner_keypair)`.
pub async fn create_dht_log(
    node: &TransportNode,
) -> Result<(DhtLog, veilid_core::KeyPair)> {
    debug!("dht: create_dht_log");
    let dht = node.dht()?;
    let result = DhtLog::create(dht.routing_context()).await;
    match &result {
        Ok((log, _)) => info!(spine_key = %log.spine_key(), "dht: DhtLog created"),
        Err(e) => warn!(error = %e, "dht: DhtLog create failed"),
    }
    result
}

/// Open a DhtLog for writing with the owner keypair.
pub async fn open_dht_log_write(
    node: &TransportNode, spine_key: &str, keypair: veilid_core::KeyPair,
) -> Result<DhtLog> {
    debug!(spine_key, "dht: open_dht_log_write");
    let dht = node.dht()?;
    let result = DhtLog::open_write(dht.routing_context(), spine_key, keypair).await;
    if let Err(ref e) = result {
        warn!(spine_key, error = %e, "dht: DhtLog open_write failed");
    }
    result
}

/// Open a DhtLog for reading only.
pub async fn open_dht_log_read(
    node: &TransportNode, spine_key: &str,
) -> Result<DhtLog> {
    debug!(spine_key, "dht: open_dht_log_read");
    let dht = node.dht()?;
    let result = DhtLog::open_read(dht.routing_context(), spine_key).await;
    if let Err(ref e) = result {
        warn!(spine_key, error = %e, "dht: DhtLog open_read failed");
    }
    result
}
