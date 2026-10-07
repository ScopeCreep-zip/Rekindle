//! Leased records (record pool), string-typed.
//!
//! The daemon cannot name Veilid types (see the string-typed surface in
//! the parent module), so it borrows records through these: every record
//! operation names a lease from the unlocked session's record pool (plan
//! C7.5).

use super::super::node::TransportNode;
use super::{parse_writer, seqs_as_opt_u64};
use crate::error::{Result, TransportError};

pub use rekindle_records::lease::LeaseId;

/// Parse a DHT record key string into a Veilid `RecordKey`.
fn parse_key(key: &str) -> Result<veilid_core::RecordKey> {
    key.parse().map_err(|e| TransportError::DhtError {
        reason: format!("invalid record key '{key}': {e}"),
    })
}

fn pool(node: &TransportNode) -> Result<std::sync::Arc<rekindle_protocol::dht::pool::RecordPool>> {
    node.require_records()
}

/// Borrow `record_key`, writable with `writer` (keypair string form) when
/// given.
pub async fn acquire_str(
    node: &TransportNode,
    record_key: &str,
    writer: Option<&str>,
) -> Result<LeaseId> {
    let key = parse_key(record_key)?;
    let writer = writer.map(parse_writer).transpose()?;
    Ok(pool(node)?.acquire(&key, writer).await?)
}

/// End a borrow; the record closes when it was the last. A no-op once the
/// session is locked (the pool closed every record).
pub async fn release(node: &TransportNode, lease: LeaseId) {
    if let Ok(pool) = pool(node) {
        pool.release(lease).await;
    }
}

/// The record key a held lease borrows.
pub fn key_of(node: &TransportNode, lease: LeaseId) -> Option<String> {
    pool(node).ok()?.key_of(lease)
}

/// Create the universal v2.0 community SMPL record (`o_cnt: 0`) from raw
/// member public keys, held writable. Returns the creator lease, the
/// record key and the owner keypair string.
pub async fn create_smpl_leased(
    node: &TransportNode,
    member_pubkeys: &[[u8; 32]],
) -> Result<(LeaseId, String, String)> {
    let schema =
        rekindle_protocol::dht::schema::community_smpl_schema(member_pubkeys).map_err(|e| {
            TransportError::RecordCreateFailed {
                reason: format!("SMPL schema: {e}"),
            }
        })?;
    let (lease, key, keypair) = pool(node)?.create(schema, None).await?;
    Ok((lease, key.to_string(), keypair.to_string()))
}

/// Create a single-owner DFLT(1) record owned by `owner` (a fresh key when
/// `None`), held writable. Returns the creator lease, the record key and
/// the owner keypair string.
pub async fn create_dflt_leased(
    node: &TransportNode,
    owner: Option<&str>,
) -> Result<(LeaseId, String, String)> {
    let owner = owner.map(parse_writer).transpose()?;
    let schema = rekindle_protocol::dht::schema::invite_secrets_dflt_schema().map_err(|e| {
        TransportError::RecordCreateFailed {
            reason: format!("DFLT schema: {e}"),
        }
    })?;
    let (lease, key, keypair) = pool(node)?.create(schema, owner).await?;
    Ok((lease, key.to_string(), keypair.to_string()))
}

/// Read a subkey of a leased record.
pub async fn get_leased(
    node: &TransportNode,
    lease: LeaseId,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>> {
    let value = pool(node)?.get(lease, subkey, force_refresh).await?;
    Ok(value.map(|v| v.data().to_vec()))
}

/// Write a subkey of a leased record as `writer`. `Ok(Some(newer))` when the
/// network already held a newer value (the compare-and-swap outcome);
/// `Ok(None)` when the value is stored or was already ours. A write that did
/// not reach consensus is an error, never a silent success.
pub async fn set_leased_str(
    node: &TransportNode,
    lease: LeaseId,
    subkey: u32,
    data: Vec<u8>,
    writer: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    use rekindle_protocol::dht::pool::SetOutcome;
    let writer = writer.map(parse_writer).transpose()?;
    match pool(node)?.set(lease, subkey, data, writer).await? {
        SetOutcome::Superseded(newer) => Ok(Some(newer.data().to_vec())),
        SetOutcome::Landed | SetOutcome::Unchanged => Ok(None),
        missed @ (SetOutcome::BelowConsensus | SetOutcome::Offline) => {
            Err(TransportError::DhtError {
                reason: format!("set: not stored ({missed:?})"),
            })
        }
    }
}

/// Local-cache sequence numbers of a leased record (no network I/O).
pub async fn inspect_leased_local_seqs(
    node: &TransportNode,
    lease: LeaseId,
) -> Result<Vec<Option<u64>>> {
    let report = pool(node)?
        .inspect(lease, None, veilid_core::DHTReportScope::Local)
        .await?;
    Ok(seqs_as_opt_u64(report.local_seqs()))
}

/// Network-confirmed sequence numbers of a leased record.
pub async fn inspect_leased_network_seqs(
    node: &TransportNode,
    lease: LeaseId,
) -> Result<Vec<Option<u64>>> {
    let report = pool(node)?
        .inspect(lease, None, veilid_core::DHTReportScope::UpdateGet)
        .await?;
    Ok(seqs_as_opt_u64(report.network_seqs()))
}

/// Indices of a leased record's subkeys that hold a value,
/// network-confirmed.
pub async fn inspect_leased_present_subkeys(
    node: &TransportNode,
    lease: LeaseId,
) -> Result<Vec<u32>> {
    let report = pool(node)?
        .inspect(lease, None, veilid_core::DHTReportScope::UpdateGet)
        .await?;
    Ok(report
        .network_seqs()
        .iter()
        .enumerate()
        .filter(|(_, seq)| seq.is_some())
        .map(|(i, _)| u32::try_from(i).unwrap_or(u32::MAX))
        .collect())
}

/// Subkeys (of `subkeys`) that hold a value in the local copy after an
/// UpdateGet inspect of a leased record: the inbox and subscription-poll
/// scan's test. Not the pool's `inspect_present` (newer on the network
/// than locally), which answers a different question.
pub async fn inspect_leased_local_present(
    node: &TransportNode,
    lease: LeaseId,
    subkeys: &[u32],
) -> Result<Vec<u32>> {
    let range: rekindle_records::lease::SubkeySet = subkeys.iter().copied().collect();
    let report = pool(node)?
        .inspect(lease, Some(range), veilid_core::DHTReportScope::UpdateGet)
        .await?;
    let listed = report.subkeys();
    Ok(report
        .local_seqs()
        .iter()
        .enumerate()
        .filter(|(_, seq)| seq.to_option().is_some())
        .filter_map(|(i, _)| listed.nth_subkey(i))
        .collect())
}

/// Watch `subkeys` of a leased record; the pool watches the union of every
/// borrower's subkeys and re-arms a dead watch while one is held.
pub async fn watch_leased(node: &TransportNode, lease: LeaseId, subkeys: &[u32]) -> Result<()> {
    Ok(pool(node)?
        .watch(lease, subkeys.iter().copied().collect())
        .await?)
}

/// Watch every subkey of a leased record (its schema's count).
pub async fn watch_all_leased(node: &TransportNode, lease: LeaseId) -> Result<()> {
    Ok(pool(node)?.watch_all(lease).await?)
}

/// Read one subkey of `record_key` with a borrow of its own: a table hit
/// when the session holds the record, else an open and a close.
pub async fn read_once_str(
    node: &TransportNode,
    record_key: &str,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>> {
    let key = parse_key(record_key)?;
    let value = pool(node)?.read_once(&key, subkey, force_refresh).await?;
    Ok(value.map(|v| v.data().to_vec()))
}
