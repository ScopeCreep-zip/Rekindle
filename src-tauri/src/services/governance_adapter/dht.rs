//! Phase 23.D.4 — DHT op bodies + SQL `recent_channel_messages`
//! reader extracted from `deps_impl.rs`. Every record operation goes
//! through the session's record pool (plan C7.5): records are borrowed
//! by lease, the pool owns the one retry layer, and errors map
//! uniformly to `GovernanceRuntimeError`.

use std::sync::Arc;

use rekindle_governance_runtime::{DhtRecordInfo, GovernanceRuntimeError, RecentMessageRow};
use rekindle_protocol::dht::pool::{RecordPool, SetOutcome};
use rekindle_protocol::dht::schema;
use rekindle_records::lease::LeaseId;
use veilid_core::DHTSchema;

use crate::db_helpers::db_call_or_default;
use crate::state_helpers;

use super::GovernanceAdapter;

fn pool(adapter: &GovernanceAdapter) -> Result<Arc<RecordPool>, GovernanceRuntimeError> {
    state_helpers::record_pool(&adapter.state).map_err(GovernanceRuntimeError::Adapter)
}

fn adapter_err(what: &str, e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("{what}: {e}"))
}

async fn create(
    adapter: &GovernanceAdapter,
    schema: DHTSchema,
    owner: Option<veilid_core::KeyPair>,
) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
    let (lease, key, keypair) = pool(adapter)?
        .create(schema, owner)
        .await
        .map_err(|e| adapter_err("create record", e))?;
    Ok(DhtRecordInfo {
        record_key: key.to_string(),
        owner_keypair: Some(keypair.to_string()),
        lease,
    })
}

pub(super) async fn create_smpl_record_impl(
    adapter: &GovernanceAdapter,
    member_pubkeys: &[[u8; 32]],
) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
    let smpl_schema = schema::community_smpl_schema(member_pubkeys)
        .map_err(|e| adapter_err("SMPL schema build failed", e))?;
    create(adapter, smpl_schema, None).await
}

pub(super) async fn create_dflt_record_impl(
    adapter: &GovernanceAdapter,
) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
    let dflt_schema = schema::invite_secrets_dflt_schema()
        .map_err(|e| adapter_err("DFLT schema build failed", e))?;
    create(adapter, dflt_schema, None).await
}

/// The HKDF-derived owner keypair (per identity, community, page) grants
/// write authority on any device with no persisted keypair. The returned
/// record key is NOT re-derivable, though — veilid mixes a random encryption
/// key into it and refuses to re-create an existing owner+schema record — so
/// the caller persists the key in the `overflow_next` chain and reuses it.
/// Create is therefore invoked at most once per page.
pub(super) async fn create_overflow_record_impl(
    adapter: &GovernanceAdapter,
    owner_keypair: String,
) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
    let kp = GovernanceAdapter::parse_writer_keypair(&owner_keypair)?;
    // Reuse the single-owner DFLT(1) schema — an overflow record holds exactly
    // one author's spilled governance page in subkey 0.
    let dflt_schema = schema::invite_secrets_dflt_schema()
        .map_err(|e| adapter_err("overflow DFLT schema build failed", e))?;
    create(adapter, dflt_schema, Some(kp)).await
}

pub(super) async fn acquire_record_impl(
    adapter: &GovernanceAdapter,
    record_key: &str,
    writer: Option<String>,
) -> Result<LeaseId, GovernanceRuntimeError> {
    let key = GovernanceAdapter::parse_record_key(record_key)?;
    let writer = writer
        .map(|w| GovernanceAdapter::parse_writer_keypair(&w))
        .transpose()?;
    pool(adapter)?
        .acquire(&key, writer)
        .await
        .map_err(|e| adapter_err("acquire record", e))
}

pub(super) async fn release_record_impl(adapter: &GovernanceAdapter, lease: LeaseId) {
    // Logged out: the pool is gone and has closed every record.
    if let Ok(pool) = pool(adapter) {
        pool.release(lease).await;
    }
}

pub(super) async fn get_dht_value_impl(
    adapter: &GovernanceAdapter,
    lease: LeaseId,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
    let value = pool(adapter)?
        .get(lease, subkey, force_refresh)
        .await
        .map_err(|e| adapter_err("get_dht_value", e))?;
    Ok(value.map(|v| v.data().to_vec()))
}

/// `Ok(Some(newer))` when the network already held a newer value (M9.5);
/// `Ok(None)` when the value is stored or was already ours. A write that
/// did not reach consensus is an error, not a silent success.
pub(super) async fn set_dht_value_impl(
    adapter: &GovernanceAdapter,
    lease: LeaseId,
    subkey: u32,
    value: Vec<u8>,
    writer: Option<String>,
) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
    let writer = writer
        .map(|w| GovernanceAdapter::parse_writer_keypair(&w))
        .transpose()?;
    match pool(adapter)?
        .set(lease, subkey, value, writer)
        .await
        .map_err(|e| adapter_err("set_dht_value", e))?
    {
        SetOutcome::Superseded(newer) => Ok(Some(newer.data().to_vec())),
        SetOutcome::Landed | SetOutcome::Unchanged => Ok(None),
        missed @ (SetOutcome::BelowConsensus | SetOutcome::Offline) => Err(
            GovernanceRuntimeError::Adapter(format!("set_dht_value: not stored ({missed:?})")),
        ),
    }
}

/// Reads `local_seqs()`, not `network_seqs()`: under
/// `DHTReportScope::Local` veilid never consults the network, so the
/// network half is all-`None` by construction.
pub(super) async fn inspect_dht_record_local_seqs_impl(
    adapter: &GovernanceAdapter,
    lease: LeaseId,
) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
    let report = pool(adapter)?
        .inspect(lease, None, veilid_core::DHTReportScope::Local)
        .await
        .map_err(|e| adapter_err("inspect Local", e))?;
    Ok(seqs_as_opt_u64(report.local_seqs()))
}

/// Preserve `ValueSeqNum`'s never-written sentinel as `None`.
///
/// `Some(0)` is a subkey written exactly once, which is why this cannot
/// collapse to a plain `u64`.
fn seqs_as_opt_u64(seqs: &[veilid_core::ValueSeqNum]) -> Vec<Option<u64>> {
    seqs.iter().map(|s| s.to_option().map(u64::from)).collect()
}

pub(super) async fn inspect_dht_record_update_get_seqs_impl(
    adapter: &GovernanceAdapter,
    lease: LeaseId,
) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
    let report = pool(adapter)?
        .inspect(lease, None, veilid_core::DHTReportScope::UpdateGet)
        .await
        .map_err(|e| adapter_err("inspect UpdateGet", e))?;
    Ok(seqs_as_opt_u64(report.network_seqs()))
}

pub(super) async fn inspect_dht_record_present_subkeys_impl(
    adapter: &GovernanceAdapter,
    lease: LeaseId,
) -> Result<Vec<u32>, GovernanceRuntimeError> {
    let report = pool(adapter)?
        .inspect(lease, None, veilid_core::DHTReportScope::UpdateGet)
        .await
        .map_err(|e| adapter_err("inspect UpdateGet", e))?;
    // `ValueSeqNum` is `Option<u32>`: `None` => subkey empty, `Some(_)` =>
    // value present (including seq 0). Keep only the populated indices.
    Ok(report
        .network_seqs()
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.to_option().and_then(|_| u32::try_from(i).ok()))
        .collect())
}

pub(super) async fn recent_channel_messages_impl(
    adapter: &GovernanceAdapter,
    community_id: &str,
    channel_id: &str,
    limit: i64,
) -> Vec<RecentMessageRow> {
    let _ = super::RECENT_MESSAGES_LIMIT;
    let Ok(owner_key) = state_helpers::current_owner_key(&adapter.state) else {
        return Vec::new();
    };
    let cid = community_id.to_string();
    let chan = channel_id.to_string();
    db_call_or_default(&adapter.pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id, sender_key, body, timestamp, mek_generation \
             FROM messages \
             WHERE owner_key = ?1 AND community_id = ?2 \
               AND conversation_type = 'channel' AND conversation_id = ?3 \
             ORDER BY timestamp DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key, cid, chan, limit], |row| {
            Ok(RecentMessageRow {
                message_id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                sender_pseudonym: row.get::<_, String>(1)?,
                body: row.get::<_, String>(2)?,
                timestamp: row.get::<_, i64>(3)?,
                mek_generation: row
                    .get::<_, Option<i64>>(4)?
                    .unwrap_or(0)
                    .max(0)
                    .cast_unsigned(),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .await
}

pub(super) async fn app_call_peer_impl(
    adapter: &GovernanceAdapter,
    target_route_blob: &[u8],
    payload: Vec<u8>,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    state_helpers::call_route_blob(&adapter.state, target_route_blob, payload)
        .await
        .map_err(GovernanceRuntimeError::Adapter)
}
