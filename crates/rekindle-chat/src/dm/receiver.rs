//! Inbound DM receive path.
//!
//! `handle_dm_subkey_change` is called when a DHT watch fires on a DM
//! SMPL record's subkey. The receiver loads session metadata to determine
//! the encryption path (1:1 vs group), decrypts, persists, and emits.
//!
//! Ignores echoes of our own writes (`subkey == my_subkey`).

use rekindle_types::dm_store::DmMessageInsert;

use crate::dm::deps::{DmDeps, DmEvent};
use crate::dm::envelope;
use crate::dm::error::DmError;
use crate::dm::sender::maybe_ratchet;

/// Handle an inbound DM subkey change. Called by the event router when
/// `WatchKind::DmSmpl` fires.
///
/// Steps:
/// 1. Load session metadata (my_subkey, is_group, initiator_public_key)
/// 2. Skip if subkey == my_subkey (own echo)
/// 3. Read raw value from DHT if not provided
/// 4. Parse envelope
/// 5. Decrypt: Triple Ratchet (1:1) or MEK (group)
/// 6. Persist message
/// 7. Emit event
/// 8. Fire ratchet trigger (group only)
pub async fn handle_dm_subkey_change(
    deps: &dyn DmDeps,
    record_key: &str,
    subkey: u32,
    raw_value: Option<&[u8]>,
) -> Result<(), DmError> {
    let meta = deps
        .store()
        .dm_get_session_meta(record_key)?
        .ok_or_else(|| DmError::SessionNotFound(record_key.into()))?;

    if subkey == meta.my_subkey {
        tracing::trace!(
            record_key = &record_key[..12.min(record_key.len())],
            subkey,
            "dm::receiver: own echo — skipping"
        );
        return Ok(());
    }

    // ── Read raw value if not provided by the watch event ────────
    let raw = match raw_value {
        Some(data) => data.to_vec(),
        None => {
            tracing::debug!(
                record_key = &record_key[..12.min(record_key.len())],
                subkey,
                "dm::receiver: watch carried no data — reading from DHT"
            );
            deps.dht_read_subkey(record_key, subkey, true)
                .await?
                .ok_or_else(|| {
                    DmError::Transport(format!(
                        "dm subkey {subkey} empty after watch fire on {record_key}"
                    ))
                })?
        }
    };

    tracing::info!(
        record_key = &record_key[..12.min(record_key.len())],
        subkey,
        raw_len = raw.len(),
        is_group = meta.is_group,
        "dm::receiver: processing inbound message"
    );

    // ── Replay detection: hash check BEFORE any decrypt ─────────
    let message_hash = {
        let mut hasher = blake3::Hasher::new();
        hasher.update(record_key.as_bytes());
        hasher.update(&subkey.to_le_bytes());
        hasher.update(&raw);
        *hasher.finalize().as_bytes()
    };
    if deps.store().is_dm_hash_known(&message_hash).unwrap_or(false) {
        tracing::trace!(
            record_key = &record_key[..12.min(record_key.len())],
            "dm::receiver: replay detected via hash — discarding"
        );
        return Ok(());
    }

    // ── Parse envelope ──────────────────────────────────────────
    let env = envelope::parse_envelope(&raw)?;
    tracing::debug!(
        record_key = &record_key[..12.min(record_key.len())],
        sequence = env.sequence,
        timestamp_ms = env.timestamp_ms,
        has_header = !env.encrypted_header.is_empty(),
        body_len = env.body.len(),
        "dm::receiver: envelope parsed"
    );

    // ── Dedup: skip if we already have this sequence in vault ───
    // The ratchet decrypt is stateful — each call irreversibly
    // advances chain keys. Re-decrypting an already-processed
    // message corrupts the session. Check the vault BEFORE decrypt.
    let existing_seq = deps.store().dm_next_sequence(
        record_key, &meta.peer_public_key,
    ).unwrap_or(1);
    if env.sequence < existing_seq {
        tracing::debug!(
            record_key = &record_key[..12.min(record_key.len())],
            envelope_seq = env.sequence,
            vault_next_seq = existing_seq,
            "dm::receiver: already-processed sequence — skipping to protect ratchet state"
        );
        return Ok(());
    }

    // ── Decrypt ─────────────────────────────────────────────────
    let body = if meta.is_group {
        let mek_bytes = deps
            .mek_cache()
            .observed_and_lookup(record_key, env.mek_generation)?;
        tracing::debug!(
            record_key = &record_key[..12.min(record_key.len())],
            mek_generation = env.mek_generation,
            "dm::receiver: group MEK decrypt"
        );
        envelope::decrypt_mek_body(&env, mek_bytes)?
    } else {
        tracing::debug!(
            record_key = &record_key[..12.min(record_key.len())],
            header_len = env.encrypted_header.len(),
            ct_len = env.body.len(),
            "dm::receiver: Triple Ratchet decrypt"
        );
        match deps
            .ratchet_decrypt(&meta.peer_public_key, &env.encrypted_header, &env.body)
            .await
        {
            Ok(plaintext) => String::from_utf8(plaintext).unwrap_or_else(|_| "[binary]".into()),
            Err(e) => {
                let is_aead = format!("{e}").contains("AEAD")
                    || format!("{e}").contains("tag verification");
                if is_aead {
                    tracing::error!(
                        record_key = &record_key[..12.min(record_key.len())],
                        peer = &meta.peer_public_key[..12.min(meta.peer_public_key.len())],
                        error = %e,
                        "dm::receiver: SESSION WEDGED — AEAD failed, ratchet state may be corrupted. \
                         This peer's DMs will fail until session recovery."
                    );
                }
                return Err(e.into());
            }
        }
    };

    // ── Persist ─────────────────────────────────────────────────
    if let Err(e) = deps.store().dm_persist_message(DmMessageInsert {
        record_key: record_key.into(),
        sender_pseudonym: meta.peer_public_key.clone(),
        body: body.clone(),
        timestamp_ms: env.timestamp_ms,
        sequence: env.sequence,
        mek_generation: env.mek_generation,
        is_self: false,
    }) {
        tracing::error!(
            record_key = &record_key[..12.min(record_key.len())],
            error = %e,
            "dm::receiver: PERSIST FAILED — ratchet state advanced but message NOT saved. \
             This message is permanently lost on restart."
        );
        return Err(e.into());
    }
    tracing::debug!(
        record_key = &record_key[..12.min(record_key.len())],
        sequence = env.sequence,
        "dm::receiver: message persisted to vault"
    );

    // Store message hash for replay detection (after successful persist)
    if let Err(e) = deps.store().store_dm_hash(&message_hash, record_key) {
        tracing::warn!(
            error = %e,
            "dm::receiver: message hash store failed — replay detection degraded"
        );
    }

    // ── Emit ────────────────────────────────────────────────────
    deps.emit_event(DmEvent::MessageReceived {
        record_key: record_key.into(),
        peer_key: meta.peer_public_key.clone(),
        sender_pseudonym: meta.peer_public_key.clone(),
        body,
        timestamp_ms: env.timestamp_ms,
        is_self: false,
    });

    tracing::info!(
        record_key = &record_key[..12.min(record_key.len())],
        sender = &meta.peer_public_key[..16.min(meta.peer_public_key.len())],
        sequence = env.sequence,
        "dm::receiver: message received, decrypted, persisted, emitted"
    );

    // ── Ratchet trigger (group only) ────────────────────────────
    if meta.is_group {
        maybe_ratchet(deps, record_key, env.sequence).await;
    }

    Ok(())
}
