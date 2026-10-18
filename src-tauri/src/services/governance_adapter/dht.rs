//! Phase 23.D.4 — DHT op bodies + SQL `recent_channel_messages`
//! reader extracted from `deps_impl.rs`. Every record operation goes
//! through the session's record pool (plan C7.5): records are borrowed
//! by lease, the pool owns the one retry layer, and errors map
//! uniformly to `GovernanceRuntimeError`.

use std::sync::Arc;

use rekindle_governance_runtime::{DhtRecordInfo, GovernanceRuntimeError, RecentMessageRow};
use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, RecordSchema, ReportScope, SetOutcome};

use crate::db_helpers::db_call;
use crate::state_helpers;

use super::GovernanceAdapter;

fn pool(adapter: &GovernanceAdapter) -> Result<Arc<dyn RecordPort>, GovernanceRuntimeError> {
    state_helpers::record_pool(&adapter.state).map_err(GovernanceRuntimeError::Adapter)
}

fn adapter_err(what: &str, e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("{what}: {e}"))
}

async fn create(
    adapter: &GovernanceAdapter,
    schema: RecordSchema,
    owner: Option<&str>,
) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
    let (lease, key, keypair) = pool(adapter)?
        .create(&schema, owner)
        .await
        .map_err(|e| adapter_err("create record", e))?;
    Ok(DhtRecordInfo {
        record_key: key,
        owner_keypair: Some(keypair),
        lease,
    })
}

pub(super) async fn create_smpl_record_impl(
    adapter: &GovernanceAdapter,
    member_pubkeys: &[[u8; 32]],
) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
    create(
        adapter,
        rekindle_records::schema::community_smpl(member_pubkeys),
        None,
    )
    .await
}

pub(super) async fn create_dflt_record_impl(
    adapter: &GovernanceAdapter,
) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
    create(adapter, rekindle_records::schema::invite_secrets(), None).await
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
    // Reuse the single-owner DFLT(1) schema — an overflow record holds exactly
    // one author's spilled governance page in subkey 0.
    create(
        adapter,
        rekindle_records::schema::invite_secrets(),
        Some(&owner_keypair),
    )
    .await
}

pub(super) async fn acquire_record_impl(
    adapter: &GovernanceAdapter,
    record_key: &str,
    writer: Option<String>,
) -> Result<LeaseId, GovernanceRuntimeError> {
    pool(adapter)?
        .acquire(record_key, writer.as_deref())
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
    Ok(value.map(|v| v.data))
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
    match pool(adapter)?
        .set(lease, subkey, value, writer.as_deref())
        .await
        .map_err(|e| adapter_err("set_dht_value", e))?
    {
        SetOutcome::Superseded(newer) => Ok(Some(newer.data)),
        SetOutcome::Landed | SetOutcome::Unchanged => Ok(None),
        missed @ (SetOutcome::BelowConsensus | SetOutcome::Offline) => Err(
            GovernanceRuntimeError::Adapter(format!("set_dht_value: not stored ({missed:?})")),
        ),
    }
}

/// Reads `local_seqs`, not `network_seqs`: a `Local` inspect never
/// consults the network, so the network half is all-`None` by construction.
pub(super) async fn inspect_dht_record_local_seqs_impl(
    adapter: &GovernanceAdapter,
    lease: LeaseId,
) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
    let report = pool(adapter)?
        .inspect(lease, None, ReportScope::Local)
        .await
        .map_err(|e| adapter_err("inspect Local", e))?;
    Ok(seqs_as_opt_u64(&report.local_seqs))
}

/// Keep never-written as `None`: `Some(0)` is a subkey written exactly
/// once, which is why this cannot collapse to a plain `u64`.
fn seqs_as_opt_u64(seqs: &[Option<u32>]) -> Vec<Option<u64>> {
    seqs.iter().map(|s| s.map(u64::from)).collect()
}

pub(super) async fn inspect_dht_record_update_get_seqs_impl(
    adapter: &GovernanceAdapter,
    lease: LeaseId,
) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
    let report = pool(adapter)?
        .inspect(lease, None, ReportScope::UpdateGet)
        .await
        .map_err(|e| adapter_err("inspect UpdateGet", e))?;
    Ok(seqs_as_opt_u64(&report.network_seqs))
}

pub(super) async fn inspect_dht_record_present_subkeys_impl(
    adapter: &GovernanceAdapter,
    lease: LeaseId,
) -> Result<Vec<u32>, GovernanceRuntimeError> {
    let report = pool(adapter)?
        .inspect(lease, None, ReportScope::UpdateGet)
        .await
        .map_err(|e| adapter_err("inspect UpdateGet", e))?;
    // `None` => subkey empty, `Some(_)` => value present (including seq 0).
    Ok(report
        .subkeys
        .iter()
        .zip(&report.network_seqs)
        .filter(|(_, seq)| seq.is_some())
        .map(|(subkey, _)| *subkey)
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
    let rows = db_call(&adapter.pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id, sender_key, body, timestamp, mek_generation \
             FROM messages \
             WHERE owner_key = ?1 \
               AND conversation_id IN (SELECT id FROM channels WHERE owner_key = ?1 AND community_id = ?2) \
               AND conversation_type = 'channel' AND conversation_id = ?3 \
             ORDER BY timestamp DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key, cid, chan, limit], |row| {
            // A row with no network message id or no stored generation
            // cannot go into a bundle (the joiner dedupes on the id and
            // decrypts under the generation), so it is left out.
            let message_id = row.get::<_, Option<String>>(0)?;
            let mek_generation = row.get::<_, Option<u64>>(4)?;
            let (Some(message_id), Some(mek_generation)) = (message_id, mek_generation) else {
                return Ok(None);
            };
            Ok(Some(RecentMessageRow {
                message_id,
                sender_pseudonym: row.get::<_, String>(1)?,
                body: row.get::<_, String>(2)?,
                timestamp: row.get::<_, i64>(3)?,
                mek_generation,
            }))
        })?;
        rows.filter_map(Result::transpose)
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .await;
    rows.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "reading recent channel messages for a bootstrap bundle failed");
        Vec::new()
    })
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
