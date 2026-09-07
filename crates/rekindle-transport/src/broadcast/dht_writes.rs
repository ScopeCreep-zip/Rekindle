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
use super::node::TransportNode;
use crate::error::{Result, TransportError};

// ── Record lifecycle ───────────────────────────────────────────────────

/// Create a DFLT DHT record (single owner, N subkeys).
pub async fn create_dflt(
    node: &TransportNode,
    subkey_count: u16,
    owner: Option<veilid_core::KeyPair>,
) -> Result<(String, Option<veilid_core::KeyPair>)> {
    debug!(
        subkey_count,
        has_owner = owner.is_some(),
        "dht: create_dflt"
    );
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
    node: &TransportNode,
    owner_subkey_count: u16,
    members: Vec<veilid_core::DHTSchemaSMPLMember>,
) -> Result<(String, Option<veilid_core::KeyPair>)> {
    debug!(
        owner_subkey_count,
        member_count = members.len(),
        "dht: create_smpl"
    );
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
    node: &TransportNode,
    record_key: &str,
    writer: veilid_core::KeyPair,
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
    node: &TransportNode,
    record_key: &str,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>> {
    debug!(record_key, subkey, force_refresh, "dht: get");
    let dht = node.dht()?;
    let result = dht::record::get(dht.routing_context(), record_key, subkey, force_refresh).await;
    match &result {
        Ok(Some(data)) => debug!(
            record_key,
            subkey,
            bytes = data.len(),
            "dht: get returned data"
        ),
        Ok(None) => debug!(record_key, subkey, "dht: get returned None"),
        Err(e) => warn!(record_key, subkey, error = %e, "dht: get failed"),
    }
    result
}

/// Write raw bytes to a specific subkey.
///
/// Returns `Ok(None)` when the write landed, or `Ok(Some(newer))` when
/// the network already held a newer value and ours was superseded —
/// the compare-and-swap signal the SMPL slot claim needs.
pub async fn set(
    node: &TransportNode,
    record_key: &str,
    subkey: u32,
    data: Vec<u8>,
    writer: Option<veilid_core::KeyPair>,
) -> Result<Option<Vec<u8>>> {
    let data_len = data.len();
    debug!(
        record_key,
        subkey,
        bytes = data_len,
        has_writer = writer.is_some(),
        "dht: set"
    );
    let dht = node.dht()?;
    let result = dht::record::set(dht.routing_context(), record_key, subkey, data, writer).await;
    match &result {
        Ok(None) => info!(record_key, subkey, bytes = data_len, "dht: set complete"),
        Ok(Some(_)) => info!(
            record_key,
            subkey,
            bytes = data_len,
            "dht: set superseded by newer network value"
        ),
        Err(e) => warn!(record_key, subkey, bytes = data_len, error = %e, "dht: set failed"),
    }
    result
}

// ── Watch ──────────────────────────────────────────────────────────────

/// Set a DHT watch on specific subkeys of a record.
pub async fn watch(node: &TransportNode, record_key: &str, subkeys: &[u32]) -> Result<bool> {
    debug!(record_key, subkey_count = subkeys.len(), "dht: watch");
    let dht = node.dht()?;
    let result = dht::record::watch(dht.routing_context(), record_key, subkeys).await;
    match &result {
        Ok(true) => info!(
            record_key,
            subkey_count = subkeys.len(),
            "dht: watch active"
        ),
        Ok(false) => warn!(record_key, "dht: watch declined by Veilid"),
        Err(e) => warn!(record_key, error = %e, "dht: watch failed"),
    }
    result
}

// ── Inspect ────────────────────────────────────────────────────────────

/// Inspect a record to get sequence numbers without fetching data.
pub async fn inspect(
    node: &TransportNode,
    record_key: &str,
    subkeys: Option<&[u32]>,
) -> Result<veilid_core::DHTRecordReport> {
    debug!(record_key, "dht: inspect");
    let dht = node.dht()?;
    let result = dht::record::inspect(dht.routing_context(), record_key, subkeys).await;
    if let Err(ref e) = result {
        warn!(record_key, error = %e, "dht: inspect failed");
    }
    result
}

/// Per-subkey sequence numbers from this node's **local** cache.
///
/// No network traffic. `ValueSeqNum::NONE` (an unwritten subkey) is
/// reported as `0` — callers that must tell "absent" from "written at
/// seq 0" want [`inspect_present_subkeys`] instead.
pub async fn inspect_local_seqs(node: &TransportNode, record_key: &str) -> Result<Vec<u64>> {
    let dht = node.dht()?;
    let report = dht::record::inspect_with_scope(
        dht.routing_context(),
        record_key,
        None,
        veilid_core::DHTReportScope::Local,
    )
    .await?;
    // `Local` scope populates `local_seqs`; `network_seqs` is empty.
    Ok(seqs_as_u64(report.local_seqs()))
}

/// Per-subkey sequence numbers confirmed against the **network**.
///
/// What a slot claim must use: the local cache can show a subkey free
/// when another member has already taken it.
pub async fn inspect_network_seqs(node: &TransportNode, record_key: &str) -> Result<Vec<u64>> {
    let dht = node.dht()?;
    let report = dht::record::inspect_with_scope(
        dht.routing_context(),
        record_key,
        None,
        veilid_core::DHTReportScope::UpdateGet,
    )
    .await?;
    Ok(seqs_as_u64(report.network_seqs()))
}

/// Indices of subkeys that currently hold a value, network-confirmed.
///
/// Preserves the "no value" vs "value at seq 0" distinction that
/// [`inspect_network_seqs`] flattens, so a cold join can fetch only the
/// occupied slots instead of 255 serial round trips.
pub async fn inspect_present_subkeys(node: &TransportNode, record_key: &str) -> Result<Vec<u32>> {
    let dht = node.dht()?;
    let report = dht::record::inspect_with_scope(
        dht.routing_context(),
        record_key,
        None,
        veilid_core::DHTReportScope::UpdateGet,
    )
    .await?;
    Ok(report
        .network_seqs()
        .iter()
        .enumerate()
        .filter(|(_, seq)| seq.is_some())
        .map(|(i, _)| u32::try_from(i).unwrap_or(u32::MAX))
        .collect())
}

/// Flatten sequence numbers to a dense `Vec<u64>`, mapping the
/// never-written sentinel (`ValueSeqNum::NONE`, internally `None`) to 0.
///
/// `ValueSeqNum` is a newtype over `Option<u32>`, so this goes through
/// `to_option()` rather than a numeric conversion — and the flattening
/// is why [`inspect_present_subkeys`] exists for callers that need to
/// tell "never written" from "written at seq 0".
fn seqs_as_u64(seqs: &[veilid_core::ValueSeqNum]) -> Vec<u64> {
    seqs.iter()
        .map(|seq| seq.to_option().map_or(0, u64::from))
        .collect()
}

// ── DhtLog (append-only log built on DHT records) ──────────────────────

/// Create a new DhtLog. Returns `(DhtLog, owner_keypair)`.
pub async fn create_dht_log(node: &TransportNode) -> Result<(DhtLog, veilid_core::KeyPair)> {
    debug!("dht: create_dht_log");
    let dht = node.dht()?;
    let result = DhtLog::create(dht.routing_context()).await;
    match &result {
        Ok((log, _)) => info!(spine_key = %log.spine_key(), "dht: DhtLog created"),
        Err(e) => warn!(error = %e, "dht: DhtLog create failed"),
    }
    result.map_err(Into::into)
}

/// Open a DhtLog for writing with the owner keypair.
pub async fn open_dht_log_write(
    node: &TransportNode,
    spine_key: &str,
    keypair: veilid_core::KeyPair,
) -> Result<DhtLog> {
    debug!(spine_key, "dht: open_dht_log_write");
    let dht = node.dht()?;
    let result = DhtLog::open_write(dht.routing_context(), spine_key, keypair).await;
    if let Err(ref e) = result {
        warn!(spine_key, error = %e, "dht: DhtLog open_write failed");
    }
    result.map_err(Into::into)
}

/// Open a DhtLog for reading only.
pub async fn open_dht_log_read(node: &TransportNode, spine_key: &str) -> Result<DhtLog> {
    debug!(spine_key, "dht: open_dht_log_read");
    let dht = node.dht()?;
    let result = DhtLog::open_read(dht.routing_context(), spine_key).await;
    if let Err(ref e) = result {
        warn!(spine_key, error = %e, "dht: DhtLog open_read failed");
    }
    result.map_err(Into::into)
}

// ── String-typed surface for hosts that cannot import veilid_core ─────
//
// `docs/architecture/daemon-cli.md` makes it a hard rule that only
// `rekindle-transport::broadcast/` and `subscriptions/` import
// `veilid_core`; `rekindle-node` and `rekindle-cli` reach Veilid only
// through this crate's public API. So the daemon cannot construct a
// `KeyPair` or name a `RecordKey`, yet it has to drive the same DHT
// operations as the Tauri host.
//
// `GovernanceRuntimeDeps` is built for exactly this: it exchanges every
// Veilid value as an opaque `String` / `Vec<u8>` ("Schwarzschild
// boundary — all Veilid types are exchanged as opaque String/Vec<u8>
// here"). These wrappers do the parsing on this side of the boundary so
// the daemon adapter can satisfy that trait without a veilid-core
// dependency. The Tauri host does its own parsing because it already
// holds a `RoutingContext` directly.

/// Parse a writer keypair in `KeyPair` string form.
fn parse_writer(s: &str) -> Result<veilid_core::KeyPair> {
    s.parse::<veilid_core::KeyPair>()
        .map_err(|e| TransportError::DhtError {
            reason: format!("invalid writer keypair: {e}"),
        })
}

/// [`open_readonly`] / [`open_writable`] behind one string-typed call.
pub async fn open_str(node: &TransportNode, record_key: &str, writer: Option<&str>) -> Result<()> {
    match writer {
        Some(w) => open_writable(node, record_key, parse_writer(w)?).await,
        None => open_readonly(node, record_key).await,
    }
}

/// [`set`] with the writer supplied as a string.
///
/// Returns the same compare-and-swap outcome: `Ok(None)` when the write
/// landed, `Ok(Some(newer))` when it was superseded.
pub async fn set_str(
    node: &TransportNode,
    record_key: &str,
    subkey: u32,
    data: Vec<u8>,
    writer: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    let writer = writer.map(parse_writer).transpose()?;
    set(node, record_key, subkey, data, writer).await
}

/// [`create_dflt`] returning the owner keypair in string form.
pub async fn create_dflt_str(
    node: &TransportNode,
    subkey_count: u16,
    owner: Option<&str>,
) -> Result<(String, Option<String>)> {
    let owner = owner.map(parse_writer).transpose()?;
    let (key, keypair) = create_dflt(node, subkey_count, owner).await?;
    Ok((key, keypair.map(|kp| kp.to_string())))
}

/// Create the universal v2.0 community SMPL record from raw member
/// public keys.
///
/// Always `o_cnt: 0` — the creation keypair owns no subkeys and is
/// discarded after genesis (the Schwarzschild principle). The returned
/// owner keypair is therefore `None` in practice; it is passed through
/// rather than dropped so the caller sees what veilid actually reported.
pub async fn create_smpl_str(
    node: &TransportNode,
    member_pubkeys: &[[u8; 32]],
) -> Result<(String, Option<String>)> {
    let members: Vec<veilid_core::DHTSchemaSMPLMember> = member_pubkeys
        .iter()
        .map(|pk| veilid_core::DHTSchemaSMPLMember {
            m_key: veilid_core::BareMemberId::new(pk),
            m_cnt: 1,
        })
        .collect();
    let (key, keypair) = create_smpl(node, 0, members).await?;
    Ok((key, keypair.map(|kp| kp.to_string())))
}

/// Derive a member slot's writer keypair from the shared slot seed,
/// returned in string form.
///
/// Wraps `rekindle_protocol`'s derivation so the daemon does not need to
/// depend on that crate purely to obtain a Veilid keypair it cannot name.
pub fn derive_slot_keypair_str(seed: &[u8; 32], slot: u32) -> Result<String> {
    rekindle_protocol::dht::community::member_registry::derive_slot_veilid_keypair(seed, slot)
        .map(|kp| kp.to_string())
        .map_err(|e| TransportError::DhtError {
            reason: format!("derive slot keypair: {e}"),
        })
}

/// Convert an Ed25519 `(public, secret)` pair into writer-keypair string
/// form, for `GovernanceRuntimeDeps::format_writer_keypair`.
pub fn format_keypair_str(ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
    let bare_pub = veilid_core::BarePublicKey::new(&ed_public);
    let bare_secret = veilid_core::BareSecretKey::new(&ed_secret);
    let pubkey = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    veilid_core::KeyPair::new_from_parts(pubkey, bare_secret).to_string()
}
