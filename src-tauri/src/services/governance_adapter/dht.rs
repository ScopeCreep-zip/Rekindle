//! Phase 23.D.4 — DHT op bodies + SQL `recent_channel_messages`
//! reader extracted from `deps_impl.rs`. All read/write/inspect ops
//! resolve the `RoutingContext` via the parent adapter's `rc()`
//! helper, then map errors uniformly to `GovernanceRuntimeError`.

use rekindle_governance_runtime::{
    DhtRecordInfo, GovernanceRuntimeError, MemberIndexRow, RecentMessageRow,
};
use rekindle_records::schema;
use veilid_core::{CRYPTO_KIND_VLD0, SetDHTValueOptions};

use crate::db_helpers::db_call_or_default;
use crate::state_helpers;

use super::GovernanceAdapter;

pub(super) async fn create_smpl_record_impl(
    adapter: &GovernanceAdapter,
    member_pubkeys: &[[u8; 32]],
) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
    let rc = adapter.rc()?;
    let smpl_schema = schema::community_smpl_schema(member_pubkeys)
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("SMPL schema build failed: {e}")))?;
    let desc = rc
        .create_dht_record(CRYPTO_KIND_VLD0, smpl_schema, None)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("create_dht_record failed: {e}")))?;
    let record_key = desc.key().to_string();
    let owner_keypair = desc
        .owner_secret()
        .map(|s| veilid_core::KeyPair::new_from_parts(desc.owner().clone(), s.value()).to_string());
    Ok(DhtRecordInfo { record_key, owner_keypair })
}

pub(super) async fn get_dht_value_impl(
    adapter: &GovernanceAdapter,
    record_key: &str,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
    let rc = adapter.rc()?;
    let key = GovernanceAdapter::parse_record_key(record_key)?;
    let value = rc
        .get_dht_value(key, subkey, force_refresh)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("get_dht_value: {e}")))?;
    Ok(value.map(|v| v.data().to_vec()))
}

pub(super) async fn set_dht_value_impl(
    adapter: &GovernanceAdapter,
    record_key: &str,
    subkey: u32,
    value: Vec<u8>,
    writer: Option<String>,
) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
    let rc = adapter.rc()?;
    let key = GovernanceAdapter::parse_record_key(record_key)?;
    let opts = match writer {
        Some(w) => Some(SetDHTValueOptions {
            writer: Some(GovernanceAdapter::parse_writer_keypair(&w)?),
            ..Default::default()
        }),
        None => None,
    };
    let outcome = rc
        .set_dht_value(key, subkey, value, opts)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("set_dht_value: {e}")))?;
    Ok(outcome.map(|v| v.data().to_vec()))
}

pub(super) async fn inspect_dht_record_local_seqs_impl(
    adapter: &GovernanceAdapter,
    record_key: &str,
) -> Result<Vec<u64>, GovernanceRuntimeError> {
    let rc = adapter.rc()?;
    let key = GovernanceAdapter::parse_record_key(record_key)?;
    let report = rc
        .inspect_dht_record(key, None, veilid_core::DHTReportScope::Local)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("inspect Local: {e}")))?;
    Ok(report
        .network_seqs()
        .iter()
        .map(|s| u64::from(s.to_option().unwrap_or(0)))
        .collect())
}

pub(super) async fn inspect_dht_record_update_get_seqs_impl(
    adapter: &GovernanceAdapter,
    record_key: &str,
) -> Result<Vec<u64>, GovernanceRuntimeError> {
    let rc = adapter.rc()?;
    let key = GovernanceAdapter::parse_record_key(record_key)?;
    let report = rc
        .inspect_dht_record(
            key,
            Some(veilid_core::ValueSubkeyRangeSet::full()),
            veilid_core::DHTReportScope::UpdateGet,
        )
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("inspect UpdateGet: {e}")))?;
    Ok(report
        .network_seqs()
        .iter()
        .map(|s| u64::from(s.to_option().unwrap_or(0)))
        .collect())
}

pub(super) async fn open_dht_record_impl(
    adapter: &GovernanceAdapter,
    record_key: &str,
    writer: Option<String>,
) -> Result<(), GovernanceRuntimeError> {
    let rc = adapter.rc()?;
    let key = GovernanceAdapter::parse_record_key(record_key)?;
    let kp = match writer {
        Some(w) => Some(GovernanceAdapter::parse_writer_keypair(&w)?),
        None => None,
    };
    let _desc = rc
        .open_dht_record(key, kp)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("open_dht_record: {e}")))?;
    Ok(())
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
    let api = state_helpers::veilid_api(&adapter.state)
        .ok_or(GovernanceRuntimeError::NotAttached)?;
    let route_id = api
        .import_remote_private_route(target_route_blob.to_vec())
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("import route: {e}")))?;
    let rc = adapter.rc()?;
    rc.app_call(veilid_core::Target::RouteId(route_id), payload)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("app_call: {e}")))
}

pub(super) async fn read_member_index_for_registry_impl(
    adapter: &GovernanceAdapter,
    registry_key: &str,
) -> Result<Vec<MemberIndexRow>, GovernanceRuntimeError> {
    use rekindle_protocol::dht::community::member_registry;
    use rekindle_protocol::dht::DHTManager;

    let rc = adapter.rc()?;
    let mgr = DHTManager::new(rc);
    let members = member_registry::read_member_index(&mgr, registry_key)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("read_member_index: {e}")))?;
    Ok(members
        .into_iter()
        .map(|m| MemberIndexRow {
            pseudonym_key_hex: m.pseudonym_key,
            subkey_index: m.subkey_index,
            role_ids: m.role_ids,
        })
        .collect())
}
