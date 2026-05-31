//! Device pairing handshake (architecture §28.4 line 3088).
//!
//! Existing device path:
//! 1. User presses "Pair another device" → `generate_pairing_session`
//!    creates a one-time pairing code, salt, and saves them to
//!    `pending_pairings` with a 5-minute expiry.
//! 2. UI shows the code (and a QR encoding the
//!    `code | salt | personal_record_key` triple) to the user.
//! 3. New device receives the code (typed or QR-scanned) and dials
//!    back via `app_call` carrying a [`PairingPayload`].
//! 4. The existing device's `app_call` handler invokes
//!    [`handle_pairing_app_call`], which verifies the code, decodes
//!    the wrapped master secret, and ships the personal record key
//!    + assigned device id back as a [`PairingAccept`].
//!
//! New device path: [`accept_pairing_payload`] takes the code + salt +
//! personal record key the user transcribed/scanned, derives the
//! pairing key, wraps this device's freshly-generated master secret
//! request, and sends it via `app_call`.

use std::sync::Arc;

use rekindle_secrets::sync_key::{generate_pairing_code, random_pairing_salt, PairingKey};
use rekindle_types::cross_device_sync::{DeviceListEntry, PairingAccept, PairingPayload};

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::state::AppState;
use crate::state_helpers;

const PAIRING_TTL_SECS: i64 = 300; // 5 minutes

/// Pre-shared data the existing device displays for transport to the
/// new device — typically rendered as a QR code that combines all
/// three fields, with the code itself also shown human-readably for
/// manual entry as a fallback.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingSession {
    pub pairing_code: String,
    pub pairing_salt_hex: String,
    pub personal_record_key: String,
    pub expires_at: i64,
    /// Architecture §28.4 — the existing device's private-route blob,
    /// hex-encoded. The new device needs this to dial the existing
    /// device via `app_call`. Encoded into the QR code alongside the
    /// pairing code + salt; empty when no route is available yet
    /// (e.g. during initial network attach).
    pub existing_device_route_blob_hex: String,
}

pub async fn generate_pairing_session(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Result<PairingSession, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let handle = super::record::open_personal_sync_record(state, pool)
        .await
        .ok_or("personal sync record not initialized — call ensure first")?;
    let pairing_code = generate_pairing_code();
    let salt = random_pairing_salt();
    let salt_hex = hex::encode(salt);
    let now = rekindle_utils::timestamp_ms_i64();
    let expires_at = now + PAIRING_TTL_SECS * 1000;
    let owner_owned = owner_key.clone();
    let code_for_db = pairing_code.clone();
    let salt_for_db = salt.to_vec();
    db_call(pool, move |conn| {
        // Garbage-collect expired sessions for this owner before insert.
        conn.execute(
            "DELETE FROM pending_pairings WHERE owner_key = ?1 AND expires_at < ?2",
            rusqlite::params![owner_owned, now],
        )?;
        conn.execute(
            "INSERT INTO pending_pairings (owner_key, pairing_code, pairing_salt, created_at, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![owner_owned, code_for_db, salt_for_db, now, expires_at],
        )?;
        Ok(())
    })
    .await?;

    let route_blob_hex = state_helpers::our_route_blob(state)
        .map(hex::encode)
        .unwrap_or_default();

    Ok(PairingSession {
        pairing_code,
        pairing_salt_hex: salt_hex,
        personal_record_key: handle.record_key,
        expires_at,
        existing_device_route_blob_hex: route_blob_hex,
    })
}

/// Existing device's `app_call` handler. The new device sends a
/// `PairingPayload`; we look the matching `pending_pairings` row up,
/// re-derive the pairing key, decrypt the wrapped master secret to
/// confirm the new device knew the code, register them in
/// `paired_devices` + the DeviceList subkey, and reply with the
/// personal record key the new device should start watching.
pub async fn handle_pairing_app_call(
    state: &Arc<AppState>,
    pool: &DbPool,
    payload: PairingPayload,
) -> Result<PairingAccept, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let now = rekindle_utils::timestamp_ms_i64();
    let owner_owned = owner_key.clone();
    let salt = payload.pairing_salt.clone();
    let row: Option<String> = db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT pairing_code FROM pending_pairings \
              WHERE owner_key = ?1 AND pairing_salt = ?2 AND expires_at > ?3",
        )?;
        let mut rows = stmt.query(rusqlite::params![owner_owned, salt, now])?;
        if let Some(r) = rows.next()? {
            return Ok(Some(r.get::<_, String>(0)?));
        }
        Ok(None)
    })
    .await;
    let pairing_code = row.ok_or("no matching pending pairing — code expired or wrong salt")?;

    let pairing_key = PairingKey::derive(&pairing_code, &payload.pairing_salt);
    let master_secret =
        pairing_key.unwrap_master_secret(&payload.wrapped_master_secret, &payload.nonce)?;
    // Verify it matches our identity — the new device claimed to be us.
    let our_secret = *state.identity_secret.lock().as_ref().ok_or("no identity")?;
    if master_secret.len() != our_secret.len()
        || master_secret.iter().zip(&our_secret).any(|(a, b)| a != b)
    {
        return Err("pairing payload master secret does not match this identity".to_string());
    }

    let handle = super::record::open_personal_sync_record(state, pool)
        .await
        .ok_or("personal sync record not initialized")?;

    let new_device_id = generate_device_id();
    let now_u64 = u64::try_from(now).unwrap_or(0);
    let entry = DeviceListEntry {
        device_id: new_device_id.clone(),
        device_public_key: hex::encode([0u8; 32]),
        display_name: payload.display_name.clone(),
        paired_at: now_u64,
        unpaired_at: None,
    };
    persist_paired_device(pool, &owner_key, &entry).await?;

    // Burn the pending row.
    let owner_owned = owner_key.clone();
    let code_for_delete = pairing_code.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "DELETE FROM pending_pairings WHERE owner_key = ?1 AND pairing_code = ?2",
            rusqlite::params![owner_owned, code_for_delete],
        )?;
        Ok(())
    })
    .await?;

    Ok(PairingAccept {
        personal_record_key: handle.record_key,
        assigned_device_id: new_device_id,
    })
}

/// New device: build a [`PairingPayload`] for the existing device
/// using `code + salt`, then ship it via `app_call`. The caller
/// supplies the route to dial the existing device on (the QR code
/// includes the existing device's route blob).
pub fn build_pairing_payload(
    state: &Arc<AppState>,
    pairing_code: &str,
    pairing_salt: &[u8],
    display_name: &str,
) -> Result<PairingPayload, String> {
    let secret = *state.identity_secret.lock().as_ref().ok_or("no identity")?;
    let pairing_key = PairingKey::derive(pairing_code, pairing_salt);
    let (wrapped, nonce) = pairing_key.wrap_master_secret(&secret)?;
    Ok(PairingPayload {
        wrapped_master_secret: wrapped,
        nonce: nonce.to_vec(),
        pairing_salt: pairing_salt.to_vec(),
        display_name: display_name.to_string(),
    })
}

async fn persist_paired_device(
    pool: &DbPool,
    owner_key: &str,
    entry: &DeviceListEntry,
) -> Result<(), String> {
    let owner_owned = owner_key.to_string();
    let did = entry.device_id.clone();
    let pk = entry.device_public_key.clone();
    let dn = entry.display_name.clone();
    let pa = entry.paired_at;
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO paired_devices \
             (owner_key, device_id, device_public_key, display_name, paired_at, unpaired_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            rusqlite::params![
                owner_owned,
                did,
                pk,
                dn,
                i64::try_from(pa).unwrap_or(i64::MAX)
            ],
        )?;
        Ok(())
    })
    .await
}

// `generate_device_id` lives in `rekindle_sync::cross_device::util`
// (centralised so `pairing.rs` + `record.rs` share one source of
// truth — pre-port these files each had a private copy).
use rekindle_sync::generate_device_id;
