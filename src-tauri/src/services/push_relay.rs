//! Mobile Push Relay client (architecture §17.3 Tier 3).
//!
//! On Tier 3, a separate headless `veilid-server` (self-hosted or
//! shared) watches DHT records on the device's behalf and forwards
//! content-free wake signals via FCM/APNs. This module is the
//! Rekindle-side client: persist registrations locally, send the
//! `RegisterPushRelay` AppCall on register, send `UnregisterPushRelay`
//! on logout. The relay daemon itself is out of scope for this crate
//! — it ships as a separate `rekindle-push-relay` binary.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::services::message_service;
use crate::state::AppState;
use crate::state_helpers;

#[allow(clippy::too_many_arguments)]
pub async fn register_with_push_relay(
    state: &Arc<AppState>,
    pool: &DbPool,
    relay_pseudonym: &str,
    device_push_token: &str,
    platform: &str,
    record_keys: &[String],
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    validate_push_relay_inputs(relay_pseudonym, device_push_token, platform, record_keys)?;
    let json =
        serde_json::to_string(record_keys).map_err(|e| format!("serialize record_keys: {e}"))?;
    let pseudonym = relay_pseudonym.to_string();
    let token = device_push_token.to_string();
    let plat = platform.to_string();
    let now = crate::db::timestamp_now();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO push_relay_registrations
                 (owner_key, relay_pseudonym, device_push_token, platform,
                  record_keys_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(owner_key, relay_pseudonym) DO UPDATE SET
                 device_push_token = excluded.device_push_token,
                 platform = excluded.platform,
                 record_keys_json = excluded.record_keys_json",
            rusqlite::params![owner_key, pseudonym, token, plat, json, now],
        )?;
        Ok(())
    })
    .await?;

    let payload = MessagePayload::RegisterPushRelay {
        device_push_token: device_push_token.to_string(),
        platform: platform.to_string(),
        record_keys: record_keys.to_vec(),
    };
    message_service::send_to_peer_raw(state, pool, relay_pseudonym, &payload).await
}

pub async fn unregister_with_push_relay(
    state: &Arc<AppState>,
    pool: &DbPool,
    relay_pseudonym: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let pseudonym = relay_pseudonym.to_string();
    let token: Option<String> = db_call(pool, {
        let owner = owner_key.clone();
        let pseudo = pseudonym.clone();
        move |conn| {
            let row = conn
                .query_row(
                    "SELECT device_push_token FROM push_relay_registrations
                     WHERE owner_key = ?1 AND relay_pseudonym = ?2",
                    rusqlite::params![owner, pseudo],
                    |r| r.get::<_, String>(0),
                )
                .ok();
            Ok(row)
        }
    })
    .await?;

    let owner_for_delete = owner_key;
    let pseudo_for_delete = pseudonym.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "DELETE FROM push_relay_registrations
             WHERE owner_key = ?1 AND relay_pseudonym = ?2",
            rusqlite::params![owner_for_delete, pseudo_for_delete],
        )?;
        Ok(())
    })
    .await?;

    if let Some(token) = token {
        let payload = MessagePayload::UnregisterPushRelay {
            device_push_token: token,
        };
        let _ = message_service::send_to_peer_raw(state, pool, &pseudonym, &payload).await;
    }
    Ok(())
}

pub async fn list_registrations(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Vec<(String, String, String)> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Vec::new();
    }
    db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT relay_pseudonym, platform, record_keys_json
             FROM push_relay_registrations WHERE owner_key = ?1",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner_key], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

/// Inbound wake — when the relay sends `WakeNotify` directly via Veilid
/// (e.g. the desktop testing path that doesn't use FCM/APNs), trigger a
/// background sync sweep of all active community records and friend
/// presence so the user sees up-to-date state. Throttled to one sweep
/// per `WAKE_NOTIFY_DEBOUNCE_SECS` because each call kicks off
/// `governance_adapter::open_community_dht_records` + `governance_adapter::rebuild_governance_from_dht`,
/// both of which spawn 30+ second DHT-bound work; back-to-back wakes
/// without throttling would saturate the network and burn battery.
pub fn handle_wake_notify(state: &Arc<AppState>, _ts: u64) {
    let now = rekindle_utils::timestamp_secs();
    {
        let mut last = state.last_wake_notify_secs.lock();
        if now.saturating_sub(*last) < WAKE_NOTIFY_DEBOUNCE_SECS {
            tracing::trace!("WakeNotify within debounce window — skipping sync sweep");
            return;
        }
        *last = now;
    }
    let state = state.clone();
    tokio::spawn(async move {
        crate::services::governance_adapter::open_community_dht_records(&state).await;
        crate::services::governance_adapter::rebuild_governance_from_dht(&state).await;
    });
}

const WAKE_NOTIFY_DEBOUNCE_SECS: u64 = 30;

/// Validate inputs before any DB write or AppCall.
///
/// - `relay_pseudonym`: must be a 64-char Ed25519 hex pubkey
///   (architecture §8.2 — pseudonym keys are Ed25519, 32 bytes hex-encoded).
/// - `device_push_token`: non-empty, max 4 KiB (FCM tokens cap ≤163 chars,
///   APNs ≤200; 4 KiB is generous and keeps a malicious self-platform
///   client from packing the field with garbage).
/// - `platform`: one of "fcm", "apns", "self".
/// - `record_keys`: at least one, each must `parse::<RecordKey>()`.
fn validate_push_relay_inputs(
    relay_pseudonym: &str,
    device_push_token: &str,
    platform: &str,
    record_keys: &[String],
) -> Result<(), String> {
    if relay_pseudonym.len() != 64 {
        return Err(format!(
            "relay_pseudonym must be 64-char hex (got {} chars)",
            relay_pseudonym.len()
        ));
    }
    let pseudonym_bytes =
        hex::decode(relay_pseudonym).map_err(|e| format!("relay_pseudonym not hex: {e}"))?;
    if pseudonym_bytes.len() != 32 {
        return Err("relay_pseudonym must decode to 32 bytes".into());
    }
    if device_push_token.is_empty() {
        return Err("device_push_token must be non-empty".into());
    }
    if device_push_token.len() > 4096 {
        return Err("device_push_token too long (max 4096 bytes)".into());
    }
    if !matches!(platform, "fcm" | "apns" | "self") {
        return Err(format!(
            "platform must be one of fcm/apns/self (got {platform})"
        ));
    }
    if record_keys.is_empty() {
        return Err("record_keys must contain at least one entry".into());
    }
    for key in record_keys {
        let _: veilid_core::RecordKey = key
            .parse()
            .map_err(|e| format!("invalid record key {key}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_push_relay_inputs;

    #[test]
    fn validate_rejects_short_pseudonym() {
        assert!(validate_push_relay_inputs("abc", "token", "self", &["VLD0:foo".into()]).is_err());
    }

    #[test]
    fn validate_rejects_unknown_platform() {
        let pseudonym = "a".repeat(64);
        assert!(
            validate_push_relay_inputs(&pseudonym, "token", "discord", &["VLD0:foo".into()])
                .is_err()
        );
    }

    #[test]
    fn validate_rejects_empty_record_keys() {
        let pseudonym = "a".repeat(64);
        assert!(validate_push_relay_inputs(&pseudonym, "token", "self", &[]).is_err());
    }

    #[test]
    fn validate_rejects_empty_token() {
        let pseudonym = "a".repeat(64);
        assert!(validate_push_relay_inputs(&pseudonym, "", "self", &["VLD0:foo".into()]).is_err());
    }
}
