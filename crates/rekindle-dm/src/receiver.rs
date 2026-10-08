//! Phase 13 — inbound DM receive path (architecture §27.1 + §5.3).
//!
//! `handle_dm_subkey_change` is called when a `VeilidValueChange` fires
//! on a DM SMPL record's subkey. The envelope carries `mek_generation`
//! so the receiver can pick the right historical key from its
//! `DmMekChain` (§5.3 line 1186 — receiver caches historical MEKs).
//!
//! Observing a higher generation forward-locks our own writer
//! generation so monotonic convergence holds. Parameterized over
//! `DmDeps` for the same reasons as the sender path.

use crate::deps::{DmDeps, DmEvent};
use crate::envelope::{decrypt_body, parse_envelope};
use crate::error::DmError;
use crate::sender::maybe_ratchet;
use crate::store::DmMessageInsert;

/// Inbound DM subkey-changed handler. Decodes the envelope, materializes
/// the MEK for the sender's generation, decrypts, persists, and emits
/// a frontend event. Ignores echoes of our own writes (`subkey == my_subkey`).
pub async fn handle_dm_subkey_change<D: DmDeps + ?Sized>(
    deps: &D,
    record_key: &str,
    subkey: u32,
    raw_value: &[u8],
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;

    let meta = deps
        .store()
        .get_session_meta(&owner_key, record_key)
        .await?
        .ok_or_else(|| DmError::SessionNotFound(record_key.to_string()))?;
    let peer_pseudonym = meta.initiator_pseudonym.clone();
    if subkey == meta.my_subkey {
        return Ok(()); // our own write echoed back; ignore
    }

    let envelope = parse_envelope(raw_value)?;

    // Materialize the MEK for the sender's generation and forward-lock
    // our own outbound generation so we never write at a lower one.
    let mek_bytes_for_gen = deps
        .mek_cache()
        .observed_and_lookup(record_key, envelope.mek_generation)?;
    let body = decrypt_body(&envelope, mek_bytes_for_gen)?;

    let timestamp_secs = i64::try_from(envelope.timestamp_ms / 1000).unwrap_or(i64::MAX);
    deps.store()
        .persist_message(
            &owner_key,
            DmMessageInsert {
                record_key: record_key.to_string(),
                sender_pseudonym: peer_pseudonym.clone(),
                body: body.clone(),
                timestamp_secs,
                sequence: envelope.sequence,
                mek_generation: envelope.mek_generation,
            },
        )
        .await?;

    deps.emit_event(DmEvent::MessageReceived {
        record_key: record_key.to_string(),
        sender_pseudonym: peer_pseudonym,
        body,
        timestamp_ms: envelope.timestamp_ms,
    });

    maybe_ratchet(deps, record_key, &owner_key, envelope.sequence).await;
    Ok(())
}
