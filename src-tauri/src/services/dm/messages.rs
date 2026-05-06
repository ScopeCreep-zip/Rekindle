//! DM message send/receive (architecture §27.1 + §5.2/§5.3 envelope).
//!
//! Outbound: encrypt with the chain's *current* generation MEK, stamp
//! that generation in the envelope, write to our SMPL subkey, persist
//! locally, emit a chat event to the frontend.
//!
//! Inbound: a `VeilidValueChange` on a DM SMPL record arrives via
//! `dht_watch::handle_value_change` → `handle_dm_subkey_change`. The
//! envelope's `mek_generation` selects which historical MEK to use;
//! the chain materializes any missing generation by deterministically
//! ratcheting forward from the highest cached lower one (architecture
//! §5.3 line 1186 — receiver caches historical MEKs).
//!
//! Ratchet trigger: per architecture §27.1 each peer ratchets after
//! 100 of *its own* writes or 24h of activity. Generations are
//! independent per writer — the wire envelope tells the receiver which
//! key to use, and observing a higher generation forward-locks our
//! own writer generation so monotonic convergence holds.
//!
//! Group DM send is currently 2-party-only; the group write path lives
//! alongside in a follow-up phase (group MEK is wrapped per-recipient
//! at invite time and rotates on member departure, not via ratchet).
use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use veilid_core::{KeyPair, RecordKey};

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::state::AppState;
use crate::state_helpers;

const DM_RATCHET_MESSAGE_INTERVAL: u64 = 100;
const DM_RATCHET_TIME_INTERVAL_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DmCiphertext {
    /// AES-GCM-encoded body.
    body: Vec<u8>,
    /// Sender-local sequence (incremented per write to our subkey).
    sequence: u64,
    /// Sender-side wall clock (ms since unix epoch).
    timestamp_ms: u64,
    /// Generation of the MEK used to encrypt `body`. Architecture §5.2
    /// line 1100: the generation is in the envelope so the receiver can
    /// pick the right historical key.
    mek_generation: u64,
}

/// Outbound: write to our subkey, persist locally, emit event.
pub async fn send_dm_message(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
    body: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }

    let row: Option<(i64, String)> = {
        let owner = owner_key.clone();
        let record = record_key.to_string();
        db_call_or_default(pool, move |conn| {
            let r = conn
                .query_row(
                    "SELECT my_subkey, initiator_pseudonym
                     FROM dms WHERE owner_key = ?1 AND record_key = ?2",
                    rusqlite::params![owner, record],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .ok();
            Ok(r)
        })
        .await
    };
    let (my_subkey_i, my_pseudonym) = row.ok_or_else(|| "dm not found".to_string())?;
    let my_subkey = u32::try_from(my_subkey_i).unwrap_or(0);

    // Pull the chain's current outbound MEK + generation. Hold the lock
    // only long enough to extract bytes; we drop before any await.
    let (mek_bytes, mek_generation) = {
        let mut cache = state.dm_mek_cache.lock();
        let chain = cache
            .get_mut(record_key)
            .ok_or_else(|| "dm mek chain not cached — accept the DM first".to_string())?;
        let (gen, mek) = chain
            .current()
            .map_err(|e| format!("dm mek chain advance failed: {e}"))?;
        (*mek.as_bytes(), gen)
    };
    let mek = MediaEncryptionKey::from_bytes(mek_bytes, mek_generation);

    let now_ms = rekindle_utils::timestamp_ms();
    let now_secs = i64::try_from(now_ms / 1000).unwrap_or(i64::MAX);

    let next_sequence: u64 = {
        let owner = owner_key.clone();
        let record = record_key.to_string();
        let pseudonym = my_pseudonym.clone();
        let prev_max: Option<i64> = db_call_or_default(pool, move |conn| {
            let prev: Option<i64> = conn
                .query_row(
                    "SELECT MAX(sequence) FROM dm_messages
                     WHERE owner_key = ?1 AND record_key = ?2 AND sender_pseudonym = ?3",
                    rusqlite::params![owner, record, pseudonym],
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            Ok(prev)
        })
        .await;
        u64::try_from(prev_max.unwrap_or(0)).unwrap_or(0) + 1
    };

    let ciphertext = mek
        .encrypt(body.as_bytes())
        .map_err(|e| format!("dm encrypt: {e}"))?;
    let payload = DmCiphertext {
        body: ciphertext,
        sequence: next_sequence,
        timestamp_ms: now_ms,
        mek_generation,
    };
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("serialize dm payload: {e}"))?;

    let (slot_seed, _initiator_pubkey, is_group) =
        load_dm_slot_seed(pool, &owner_key, record_key).await?;
    if is_group {
        return Err("group dm send not yet wired".into());
    }
    let slot_signing = rekindle_secrets::derive::derive_slot_keypair(&slot_seed, my_subkey)
        .map_err(|e| format!("derive slot keypair: {e}"))?;
    let pub_bytes = slot_signing.verifying_key().to_bytes();
    let secret_bytes = slot_signing.to_bytes();
    let veilid_pub = veilid_core::PublicKey::new(
        veilid_core::CRYPTO_KIND_VLD0,
        veilid_core::BarePublicKey::new(&pub_bytes),
    );
    let veilid_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_keypair = KeyPair::new_from_parts(veilid_pub, veilid_secret);

    let rc = state_helpers::safe_routing_context(state)
        .ok_or_else(|| "no routing context".to_string())?;
    let record_key_typed = record_key
        .parse::<RecordKey>()
        .map_err(|e| format!("invalid record key: {e}"))?;
    let _ = rc
        .open_dht_record(record_key_typed.clone(), Some(veilid_keypair))
        .await
        .map_err(|e| format!("open dm record writable: {e}"))?;
    rc.set_dht_value(record_key_typed, my_subkey, payload_bytes, None)
        .await
        .map_err(|e| format!("write dm subkey: {e}"))?;

    persist_dm_message(
        pool,
        &owner_key,
        record_key,
        &my_pseudonym,
        body,
        now_secs,
        next_sequence,
        mek_generation,
    )
    .await?;

    if let Some(app_handle) = state_helpers::app_handle(state) {
        let _ = app_handle.emit(
            "chat-event",
            &ChatEvent::MessageReceived {
                from: my_pseudonym.clone(),
                body: body.to_string(),
                decryption_failed: false,
                automod_blurred: false,
                timestamp: now_ms,
                conversation_id: record_key.to_string(),
                server_message_id: None,
                reply_to_id: None,
                sender_display_name: None,
            },
        );
    }

    maybe_ratchet(state, pool, record_key, &owner_key, next_sequence).await;

    Ok(())
}

/// Inbound: a watch fired on a DM SMPL record. Decode, decrypt, persist, emit.
pub async fn handle_dm_subkey_change(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
    subkey: u32,
    raw_value: &[u8],
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }

    let row: Option<(i64, String, bool)> = {
        let owner = owner_key.clone();
        let record = record_key.to_string();
        db_call_or_default(pool, move |conn| {
            let r = conn
                .query_row(
                    "SELECT my_subkey, initiator_pseudonym, is_group
                     FROM dms WHERE owner_key = ?1 AND record_key = ?2",
                    rusqlite::params![owner, record],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)? != 0,
                        ))
                    },
                )
                .ok();
            Ok(r)
        })
        .await
    };
    let (my_subkey_i, peer_pseudonym, is_group) =
        row.ok_or_else(|| "dm row not found".to_string())?;
    let my_subkey = u32::try_from(my_subkey_i).unwrap_or(0);
    if subkey == my_subkey {
        return Ok(()); // our own write echoed back; ignore
    }
    let _ = is_group;

    let payload: DmCiphertext = serde_json::from_slice(raw_value)
        .map_err(|e| format!("invalid dm payload: {e}"))?;

    // Materialize the MEK for the sender's generation, then forward-lock
    // our own outbound generation so we never write at a lower one.
    let mek_bytes_for_gen = {
        let mut cache = state.dm_mek_cache.lock();
        let chain = cache
            .get_mut(record_key)
            .ok_or_else(|| "dm mek chain not cached".to_string())?;
        chain
            .observed_generation(payload.mek_generation)
            .map_err(|e| format!("dm chain materialize: {e}"))?;
        let mek = chain
            .for_generation(payload.mek_generation)
            .map_err(|e| format!("dm chain lookup: {e}"))?;
        *mek.as_bytes()
    };
    let mek = MediaEncryptionKey::from_bytes(mek_bytes_for_gen, payload.mek_generation);
    let plaintext_bytes = mek
        .decrypt(&payload.body)
        .map_err(|e| format!("dm decrypt: {e}"))?;
    let body = String::from_utf8(plaintext_bytes)
        .map_err(|e| format!("dm body not utf8: {e}"))?;

    let timestamp_secs = i64::try_from(payload.timestamp_ms / 1000).unwrap_or(i64::MAX);
    persist_dm_message(
        pool,
        &owner_key,
        record_key,
        &peer_pseudonym,
        &body,
        timestamp_secs,
        payload.sequence,
        payload.mek_generation,
    )
    .await?;

    if let Some(app_handle) = state_helpers::app_handle(state) {
        let _ = app_handle.emit(
            "chat-event",
            &ChatEvent::MessageReceived {
                from: peer_pseudonym.clone(),
                body: body.clone(),
                decryption_failed: false,
                automod_blurred: false,
                timestamp: payload.timestamp_ms,
                conversation_id: record_key.to_string(),
                server_message_id: None,
                reply_to_id: None,
                sender_display_name: None,
            },
        );
    }

    maybe_ratchet(state, pool, record_key, &owner_key, payload.sequence).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn persist_dm_message(
    pool: &DbPool,
    owner_key: &str,
    record_key: &str,
    sender_pseudonym: &str,
    body: &str,
    timestamp: i64,
    sequence: u64,
    mek_generation: u64,
) -> Result<(), String> {
    let owner = owner_key.to_string();
    let record = record_key.to_string();
    let sender = sender_pseudonym.to_string();
    let body_owned = body.to_string();
    let seq_i = i64::try_from(sequence).unwrap_or(i64::MAX);
    let gen_i = i64::try_from(mek_generation).unwrap_or(i64::MAX);
    let now = crate::db::timestamp_now();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO dm_messages
                (owner_key, record_key, sender_pseudonym, body, timestamp,
                 sequence, mek_generation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![owner, record, sender, body_owned, timestamp, seq_i, gen_i],
        )?;
        conn.execute(
            "UPDATE dms SET last_message_at = ?3 WHERE owner_key = ?1 AND record_key = ?2",
            rusqlite::params![owner, record, now],
        )?;
        Ok(())
    })
    .await
}

async fn load_dm_slot_seed(
    pool: &DbPool,
    owner_key: &str,
    record_key: &str,
) -> Result<([u8; 32], String, bool), String> {
    let owner = owner_key.to_string();
    let record = record_key.to_string();
    let row: Option<(String, String, bool)> = db_call_or_default(pool, move |conn| {
        let row = conn
            .query_row(
                "SELECT slot_seed_hex, initiator_public_key, is_group
                 FROM dms WHERE owner_key = ?1 AND record_key = ?2",
                rusqlite::params![owner, record],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    ))
                },
            )
            .ok();
        Ok(row)
    })
    .await;
    let (seed_hex, initiator, is_group) = row.ok_or_else(|| "dm not found".to_string())?;
    let seed_bytes: [u8; 32] = hex::decode(&seed_hex)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes".to_string())?;
    Ok((seed_bytes, initiator, is_group))
}

/// Forward-secure ratchet trigger (architecture §27.1): each peer
/// advances its own generation after 100 messages it has sent or
/// 24 hours of activity, whichever first. Receivers materialize the
/// new generation lazily via `DmMekChain` on the next inbound message.
async fn maybe_ratchet(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
    owner_key: &str,
    last_sequence: u64,
) {
    let owner = owner_key.to_string();
    let record = record_key.to_string();
    let oldest_recent_ts: Option<i64> = db_call_or_default(pool, move |conn| {
        let row: Option<i64> = conn
            .query_row(
                "SELECT MIN(timestamp) FROM dm_messages
                 WHERE owner_key = ?1 AND record_key = ?2
                   AND sequence > (
                     SELECT COALESCE(MAX(sequence), 0) - ?3 FROM dm_messages
                     WHERE owner_key = ?1 AND record_key = ?2
                   )",
                rusqlite::params![owner, record, DM_RATCHET_MESSAGE_INTERVAL.cast_signed()],
                |r| r.get(0),
            )
            .ok();
        Ok(row)
    })
    .await;

    let now_secs = i64::try_from(rekindle_utils::timestamp_ms() / 1000).unwrap_or(i64::MAX);
    let time_trigger = oldest_recent_ts
        .is_some_and(|ts| now_secs - ts >= DM_RATCHET_TIME_INTERVAL_SECS);
    let count_trigger =
        last_sequence > 0 && last_sequence.is_multiple_of(DM_RATCHET_MESSAGE_INTERVAL);
    if !(time_trigger || count_trigger) {
        return;
    }

    let new_gen = {
        let mut cache = state.dm_mek_cache.lock();
        let Some(chain) = cache.get_mut(record_key) else {
            return;
        };
        match chain.advance() {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(error = %e, "dm chain advance failed");
                return;
            }
        }
    };

    let owner = owner_key.to_string();
    let record = record_key.to_string();
    let gen_i = i64::try_from(new_gen).unwrap_or(i64::MAX);
    let _ = db_call(pool, move |conn| {
        conn.execute(
            "UPDATE dms SET mek_generation = ?3
             WHERE owner_key = ?1 AND record_key = ?2",
            rusqlite::params![owner, record, gen_i],
        )?;
        Ok(())
    })
    .await;
}
