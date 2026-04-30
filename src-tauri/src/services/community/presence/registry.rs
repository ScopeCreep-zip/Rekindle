use std::sync::Arc;

use rekindle_protocol::dht::DHTManager;
use tauri::Manager;

use crate::state::AppState;
use crate::state_helpers;

use super::current_presence_status;

fn presence_event_id(event_id: &str) -> rekindle_types::id::EventId {
    let hash = blake3::hash(event_id.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    rekindle_types::id::EventId(bytes)
}

pub(super) async fn ensure_registry_open(
    state: &Arc<AppState>,
    community_id: &str,
    mgr: &DHTManager,
    registry_key: &str,
) -> Result<(), String> {
    let records_open = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|c| c.open_community_records.records_open)
    };
    if records_open {
        return Ok(());
    }

    let (registry_kp, slot_kp) = {
        let communities = state.communities.read();
        let c = communities.get(community_id);
        (
            c.and_then(|c| c.registry_owner_keypair.clone()),
            c.and_then(|c| c.slot_keypair.clone()),
        )
    };
    let writer_kp = registry_kp.or(slot_kp);
    let opened = if let Some(ref kp_str) = writer_kp {
        if let Ok(kp) = kp_str.parse::<veilid_core::KeyPair>() {
            mgr.open_record_writable(registry_key, kp).await.is_ok()
        } else {
            false
        }
    } else {
        false
    };
    if !opened {
        mgr.open_record(registry_key)
            .await
            .map_err(|e| format!("presence_poll: failed to open registry: {e}"))?;
    }
    let registry_key_owned = registry_key.to_string();
    state_helpers::track_open_records(state, std::slice::from_ref(&registry_key_owned));
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.open_community_records.registry_key = Some(registry_key.to_string());
            cs.open_community_records.registry_writer = writer_kp;
            cs.open_community_records.records_open = true;
        }
    }
    tracing::debug!(community = %community_id, "presence_poll: re-opened registry after restart");
    Ok(())
}

pub(super) async fn write_our_presence(
    state: &Arc<AppState>,
    community_id: &str,
    rc: &veilid_core::RoutingContext,
    registry_key: &str,
    my_pseudonym: &str,
    my_subkey_index: Option<u32>,
    slot_keypair_str: Option<&String>,
    has_slot_seed: bool,
    history_ranges: Vec<rekindle_types::presence::HistoryRange>,
) {
    if let (Some(subkey_idx), Some(kp_str)) = (my_subkey_index, slot_keypair_str) {
        let our_route_blob = state_helpers::our_route_blob(state);
        if our_route_blob.is_none() {
            tracing::warn!(
                community = %community_id,
                "presence_poll_tick: our_route_blob is None — peers cannot reach us"
            );
        }
        let event_rsvps = {
            let communities = state.communities.read();
            communities
                .get(community_id)
                .map(|community| {
                    community
                        .my_event_rsvps
                        .iter()
                        .map(|(event_id, status)| rekindle_types::presence::EventRSVP {
                            event_id: presence_event_id(event_id),
                            status: status.clone(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let presence = rekindle_types::presence::MemberPresence {
            pseudonym_key: rekindle_types::id::PseudonymKey(
                hex::decode(my_pseudonym)
                    .ok()
                    .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                    .unwrap_or([0u8; 32]),
            ),
            display_name: Some(state_helpers::identity_display_name(state)),
            status: current_presence_status(state).into(),
            route_blob: our_route_blob.unwrap_or_default(),
            last_heartbeat: rekindle_utils::timestamp_secs(),
            event_rsvps,
            history_ranges,
            ..Default::default()
        };
        if let (Ok(writer_kp), Ok(reg_key)) = (
            kp_str.parse::<veilid_core::KeyPair>(),
            registry_key.parse::<veilid_core::RecordKey>(),
        ) {
            let write_opts = veilid_core::SetDHTValueOptions {
                writer: Some(writer_kp),
                ..Default::default()
            };
            if let Err(e) = rc
                .set_dht_value(
                    reg_key,
                    subkey_idx,
                    serde_json::to_vec(&presence).unwrap_or_default(),
                    Some(write_opts),
                )
                .await
            {
                tracing::debug!(
                    community = %community_id,
                    subkey = subkey_idx,
                    error = %e,
                    "failed to write presence to registry"
                );
            }
        }
    } else {
        tracing::warn!(
            community = %community_id,
            has_slot_keypair = slot_keypair_str.is_some(),
            has_subkey_index = my_subkey_index.is_some(),
            has_slot_seed,
            "cannot write presence — missing slot keypair or subkey index"
        );
    }
}

pub(super) fn persist_discovered_registry_members(
    state: &Arc<AppState>,
    community_id: &str,
    discovered_members: &[(u32, rekindle_types::presence::MemberPresence)],
    member_roles: &std::collections::HashMap<String, Vec<u32>>,
    banned_members: &std::collections::HashSet<String>,
) {
    let app_handle = { state.app_handle.read().clone() };
    let Some(app_handle) = app_handle else { return };
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return;
    };
    let cid = community_id.to_string();
    let rows: Vec<(String, Option<String>, String, i64)> = discovered_members
        .iter()
        .map(|(subkey, presence)| {
            let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
            let role_ids_json = serde_json::to_string(
                member_roles
                    .get(&pseudonym_hex)
                    .cloned()
                    .unwrap_or_else(|| vec![0])
                    .as_slice(),
            )
            .unwrap_or_else(|_| "[0]".to_string());
            (
                pseudonym_hex,
                presence.display_name.clone(),
                role_ids_json,
                i64::from(*subkey),
            )
        })
        .collect();
    let banned_rows: Vec<String> = banned_members.iter().cloned().collect();
    let joined_at = crate::db::timestamp_now();
    crate::db_helpers::db_fire(
        pool.inner(),
        "persist discovered registry members",
        move |conn| {
            for banned in &banned_rows {
                conn.execute(
                    "DELETE FROM community_members WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                    rusqlite::params![owner_key, cid, banned],
                )?;
            }
            for (pseudonym_key, display_name, role_ids_json, subkey_index) in &rows {
                conn.execute(
                    "INSERT INTO community_members \
                 (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at, subkey_index) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(owner_key, community_id, pseudonym_key) DO UPDATE SET \
                   display_name = excluded.display_name, \
                   role_ids = excluded.role_ids, \
                   subkey_index = excluded.subkey_index",
                    rusqlite::params![
                        owner_key,
                        cid,
                        pseudonym_key,
                        display_name,
                        role_ids_json,
                        joined_at,
                        subkey_index,
                    ],
                )?;
            }
            Ok(())
        },
    );
}
