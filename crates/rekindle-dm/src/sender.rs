//! Phase 13 — outbound DM send path (architecture §27.1).
//!
//! `send_dm_message` is the full sender pipeline: pull the conversation
//! metadata from the store, materialize the current outbound MEK from
//! the chain, build the envelope, derive the slot keypair, open the
//! DHT record writable, write the encrypted body to our subkey,
//! persist locally, emit a frontend echo, and fire the ratchet trigger
//! if the 100-message / 24-hour threshold is met.
//!
//! Parameterized over `DmDeps` so this crate never imports
//! `veilid-core`, `AppState`, or `tauri::AppHandle` — every operation
//! that needs those goes through a trait method whose adapter
//! implementation lives in `src-tauri/services/dm_adapter.rs`.

use crate::deps::{DmDeps, DmEvent};
use crate::envelope::{build_envelope, DM_RATCHET_MESSAGE_INTERVAL, DM_RATCHET_TIME_INTERVAL_SECS};
use crate::error::DmError;
use crate::store::DmMessageInsert;

/// Outbound 1:1 DM send. Errors map to `DmError`; 2-party only — group
/// DM send is gated below (`DmError::InvalidInput`) until the group
/// write path lands in a follow-up phase.
pub async fn send_dm_message<D: DmDeps + ?Sized>(
    deps: &D,
    record_key: &str,
    body: &str,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;

    let meta = deps
        .store()
        .get_session_meta(&owner_key, record_key)
        .await?
        .ok_or_else(|| DmError::SessionNotFound(record_key.to_string()))?;
    if meta.is_group {
        return Err(DmError::InvalidInput("group dm send not yet wired".into()));
    }
    let my_subkey = meta.my_subkey;
    let my_pseudonym = meta.initiator_pseudonym.clone();

    // Pull the chain's current outbound MEK + generation. Adapter
    // implementation holds the lock only long enough to copy bytes.
    let (mek_bytes, mek_generation) = deps.mek_cache().current(record_key)?;

    let now_ms = rekindle_utils::timestamp_ms();
    let now_secs = i64::try_from(now_ms / 1000).unwrap_or(i64::MAX);

    let next_sequence = deps
        .store()
        .next_sequence_for_sender(&owner_key, record_key, &my_pseudonym)
        .await?;

    let payload_bytes = build_envelope(mek_bytes, mek_generation, body, next_sequence, now_ms)?;

    let slot_signing = rekindle_secrets::derive::derive_slot_keypair(&meta.slot_seed, my_subkey)
        .map_err(|e| DmError::InvalidInput(format!("derive slot keypair: {e}")))?;
    let slot_public = slot_signing.verifying_key().to_bytes();
    let slot_secret = slot_signing.to_bytes();

    deps.dht_open_record(record_key, Some((slot_secret, slot_public)))
        .await?;
    deps.dht_write_subkey(
        record_key,
        my_subkey,
        payload_bytes,
        (slot_secret, slot_public),
    )
    .await?;

    deps.store()
        .persist_message(
            &owner_key,
            DmMessageInsert {
                record_key: record_key.to_string(),
                sender_pseudonym: my_pseudonym.clone(),
                body: body.to_string(),
                timestamp_secs: now_secs,
                sequence: next_sequence,
                mek_generation,
            },
        )
        .await?;

    deps.emit_event(DmEvent::MessageReceived {
        record_key: record_key.to_string(),
        sender_pseudonym: my_pseudonym,
        body: body.to_string(),
        timestamp_ms: now_ms,
    });

    maybe_ratchet(deps, record_key, &owner_key, next_sequence).await;
    Ok(())
}

/// Forward-secure ratchet trigger (architecture §27.1): each peer
/// advances its own generation after 100 messages it has sent or 24
/// hours of activity, whichever first. Receivers materialize the new
/// generation lazily via `DmMekChain` on the next inbound message.
pub(crate) async fn maybe_ratchet<D: DmDeps + ?Sized>(
    deps: &D,
    record_key: &str,
    owner_key: &str,
    last_sequence: u64,
) {
    let oldest_ts = deps
        .store()
        .oldest_recent_message_ts(
            owner_key,
            record_key,
            i64::try_from(DM_RATCHET_MESSAGE_INTERVAL).unwrap_or(i64::MAX),
        )
        .await
        .ok()
        .flatten();

    let now_secs = i64::try_from(rekindle_utils::timestamp_ms() / 1000).unwrap_or(i64::MAX);
    let time_trigger = oldest_ts.is_some_and(|ts| now_secs - ts >= DM_RATCHET_TIME_INTERVAL_SECS);
    let count_trigger =
        last_sequence > 0 && last_sequence.is_multiple_of(DM_RATCHET_MESSAGE_INTERVAL);
    if !(time_trigger || count_trigger) {
        return;
    }

    let new_gen = match deps.mek_cache().advance(record_key) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!(error = %e, record_key, "dm chain advance failed");
            return;
        }
    };

    let gen_u32 = u32::try_from(new_gen).unwrap_or(u32::MAX);
    if let Err(e) = deps
        .store()
        .update_mek_generation(owner_key, record_key, gen_u32)
        .await
    {
        tracing::warn!(error = %e, record_key, "persist mek generation failed");
    }
}
