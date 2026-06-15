//! Outbound DM send path.
//!
//! `send_dm_message` is the full sender pipeline: pull conversation
//! metadata, encrypt (Triple Ratchet for 1:1, MEK for group), derive
//! slot keypair, write to SMPL subkey, persist locally, emit event,
//! fire ratchet trigger if threshold met.

use rekindle_types::dm_store::DmMessageInsert;

use crate::dm::deps::{DmDeps, DmEvent};
use crate::dm::envelope;
use crate::dm::error::DmError;
use crate::dm::mek_chain::{DM_RATCHET_MESSAGE_INTERVAL, DM_RATCHET_TIME_INTERVAL_SECS};
use crate::time::timestamp_ms;

/// Send a DM message to a conversation identified by `record_key`.
///
/// Branches on `meta.is_group`:
/// - 1:1: `deps.ratchet_encrypt` → Triple Ratchet (per-message forward secrecy)
/// - group: `deps.mek_cache().current` → AES-256-GCM via MEK
///
/// Both paths write to the same SMPL subkey via the same slot keypair.
pub async fn send_dm_message(
    deps: &dyn DmDeps,
    record_key: &str,
    body: &str,
) -> Result<(), DmError> {
    let meta = deps
        .store()
        .dm_get_session_meta(record_key)?
        .ok_or_else(|| DmError::SessionNotFound(record_key.into()))?;

    let now_ms = timestamp_ms();
    let our_identity = deps.identity_public_key_hex()
        .unwrap_or_else(|_| "self".into());

    let sequence = deps
        .store()
        .dm_next_sequence(record_key, &our_identity)?;

    tracing::info!(
        record_key = &record_key[..20.min(record_key.len())],
        is_group = meta.is_group,
        sequence,
        "dm::sender: encrypting message"
    );

    // ── Encrypt ─────────────────────────────────────────────────
    let envelope_bytes = if meta.is_group {
        let (mek_bytes, mek_gen) = deps.mek_cache().current(record_key)?;
        tracing::debug!(
            record_key = &record_key[..12.min(record_key.len())],
            mek_generation = mek_gen,
            "dm::sender: group MEK encrypt"
        );
        envelope::build_mek_envelope(mek_bytes, mek_gen, body, sequence, now_ms)?
    } else {
        let (encrypted_header, ciphertext) = deps
            .ratchet_encrypt(&meta.peer_public_key, body.as_bytes())
            .await?;
        tracing::debug!(
            record_key = &record_key[..12.min(record_key.len())],
            header_len = encrypted_header.len(),
            ct_len = ciphertext.len(),
            "dm::sender: Triple Ratchet encrypt"
        );
        envelope::build_ratchet_envelope(encrypted_header, ciphertext, sequence, now_ms)?
    };

    // ── Derive slot keypair, write to SMPL subkey ───────────────
    let slot_kp = rekindle_identity::derive_slot_keypair(&meta.slot_seed, meta.my_subkey)
        .map_err(|e| DmError::InvalidInput(format!("slot keypair: {e}")))?;
    let slot_pub = slot_kp.public_key_bytes();
    let slot_secret = {
        let mut ikm = Vec::with_capacity(36);
        ikm.extend_from_slice(&meta.slot_seed);
        ikm.extend_from_slice(&meta.my_subkey.to_le_bytes());
        blake3::derive_key(rekindle_identity::derivation_tags::SLOT_KEYPAIR, &ikm)
    };
    let writer_keypair = (slot_secret, slot_pub);

    deps.dht_write_subkey(record_key, meta.my_subkey, envelope_bytes, writer_keypair)
        .await?;

    tracing::info!(
        record_key = &record_key[..12.min(record_key.len())],
        subkey = meta.my_subkey,
        sequence,
        "dm::sender: DHT write complete"
    );

    // ── Persist locally ─────────────────────────────────────────
    deps.store().dm_persist_message(DmMessageInsert {
        record_key: record_key.into(),
        sender_pseudonym: our_identity.clone(),
        body: body.into(),
        timestamp_ms: now_ms,
        sequence,
        mek_generation: 0,
        is_self: true,
    })?;

    // ── Emit local echo ─────────────────────────────────────────
    deps.emit_event(DmEvent::MessageReceived {
        record_key: record_key.into(),
        peer_key: meta.peer_public_key.clone(),
        sender_pseudonym: our_identity,
        body: body.into(),
        timestamp_ms: now_ms,
        is_self: true,
    });

    // ── Ratchet trigger (group only — 1:1 ratchets per-message) ──
    if meta.is_group {
        maybe_ratchet(deps, record_key, sequence).await;
    }

    Ok(())
}

/// Forward-secure ratchet trigger: advance the group MEK chain after
/// 100 messages or 24 hours, whichever comes first. Receivers
/// materialize the new generation lazily via `DmMekChain` on the
/// next inbound message.
pub(crate) async fn maybe_ratchet(deps: &dyn DmDeps, record_key: &str, last_sequence: u64) {
    let oldest_ts = deps
        .store()
        .dm_oldest_recent_ts(record_key, DM_RATCHET_MESSAGE_INTERVAL as i64)
        .ok()
        .flatten();

    let now_secs = i64::try_from(timestamp_ms() / 1000).unwrap_or(i64::MAX);
    let time_trigger =
        oldest_ts.is_some_and(|ts| now_secs - ts >= DM_RATCHET_TIME_INTERVAL_SECS);
    let count_trigger =
        last_sequence > 0 && last_sequence % DM_RATCHET_MESSAGE_INTERVAL == 0;

    if !(time_trigger || count_trigger) {
        return;
    }

    tracing::info!(
        record_key = &record_key[..12.min(record_key.len())],
        time_trigger,
        count_trigger,
        last_sequence,
        "dm::sender: ratchet trigger fired"
    );

    let new_gen = match deps.mek_cache().advance(record_key) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!(error = %e, "dm::sender: MEK chain advance failed");
            return;
        }
    };
    let gen_u32 = u32::try_from(new_gen).unwrap_or(u32::MAX);
    if let Err(e) = deps.store().dm_update_mek_generation(record_key, gen_u32) {
        tracing::warn!(error = %e, "dm::sender: persist mek generation failed");
    }
}
