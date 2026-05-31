//! Phase 17 — distribute MEK + wait_for_rotation_slot orchestration.
//!
//! The `distribute_mek` body wraps the new MEK under each recipient's
//! pseudonym public key (X25519 ECDH + HKDF-SHA256 + ChaCha20-Poly1305
//! via `rekindle_crypto::group::mek_distribution::wrap_mek`), encodes
//! a `MekTransfer` control envelope, and dispatches per-recipient via
//! `MekDistributeDeps::broadcast_to_peer`. Each app_call reply is
//! inspected for `MekTransferAck` variant matching the sent
//! generation; mismatches are warned but not hard-failed because the
//! next rotation will recover.
//!
//! `wait_for_rotation_slot` implements the cascade fall-through delay:
//! the primary rotator (cascade_index 0) returns immediately; level N
//! sleeps `cascade_delay(N)` and bails out if any peer already
//! advanced the generation (we yielded the rotation).

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_crypto::group::mek_distribution::wrap_mek;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::id::PseudonymKey;

use crate::deps::{MekDistributeDeps, RotationRecipient};
use crate::election::cascade_delay;
use crate::error::MekRotationError;
use crate::event::MekRotationEvent;

/// Wait until it's our turn to attempt the rotation at `cascade_index`,
/// or yield if a peer already rotated.
///
/// `channel_id = None` means community-wide MEK rotation (e.g. text
/// channel after a member departure); `Some(channel)` means the
/// channel-scoped MEK (e.g. voice channel membership change).
///
/// Returns `Some(higher_priority_candidates)` if we should still
/// proceed (the slice we should NOT include in the recipient set when
/// distributing — they were the rotators before us, but they failed).
/// Returns `None` if we should not rotate (generation already advanced
/// or we aren't in the candidate list).
pub async fn wait_for_rotation_slot<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    channel_id: Option<&str>,
    candidates: &[PseudonymKey],
    initial_generation: u64,
) -> Option<Vec<PseudonymKey>> {
    let me = deps.my_pseudonym(community_id)?;
    let index = candidates.iter().position(|candidate| candidate == &me)?;

    if index > 0 {
        tokio::time::sleep(cascade_delay(index)).await;
        // Community-wide rotation: there's no per-channel generation
        // counter in the cache (the cache is keyed by (community,
        // channel)). For the None case we skip the LWW check — the
        // governance-state MEKGenerationBump entry serializes
        // community-wide rotations CRDT-style instead.
        if let Some(channel) = channel_id {
            if deps.cache().current_generation(community_id, channel) > initial_generation {
                return None;
            }
        }
    }

    Some(candidates.iter().take(index).cloned().collect())
}

/// Encrypt + dispatch the new MEK to each recipient. Emits
/// `RotationStarted` before the first send and `RotationComplete`
/// after the last successful one.
///
/// `recipients` should exclude our own pseudonym (the rotator); the
/// fn additionally filters out any recipient whose pseudonym matches
/// `my_pseudonym(community_id)` as a defensive measure.
pub async fn distribute_mek<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    channel_id: Option<&str>,
    new_mek: &MediaEncryptionKey,
    recipients: &[RotationRecipient],
) -> Result<(), MekRotationError> {
    let secret = deps.identity_secret().ok_or(MekRotationError::IdentityNotLoaded)?;
    let my_signing_key =
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
    let my_pseudonym = deps
        .my_pseudonym(community_id)
        .ok_or_else(|| MekRotationError::PseudonymMissing(community_id.to_string()))?;
    let my_pseudonym_hex = hex::encode(my_pseudonym.0);
    let generation = new_mek.generation();
    let event_channel = channel_id.unwrap_or("").to_string();

    deps.emit_event(MekRotationEvent::RotationStarted {
        community_id: community_id.to_string(),
        channel_id: event_channel.clone(),
        new_generation: generation,
        initiator_pseudonym_hex: my_pseudonym_hex.clone(),
    });

    for recipient in recipients {
        if recipient.pseudonym_hex == my_pseudonym_hex {
            // Defensive: should already have been excluded.
            continue;
        }
        let recipient_pseudo = pseudonym_from_hex(&recipient.pseudonym_hex)
            .ok_or_else(|| MekRotationError::InvalidInput(format!(
                "invalid recipient pseudonym: {}",
                recipient.pseudonym_hex
            )))?;
        let wrapped = wrap_mek(&my_signing_key, &recipient_pseudo.0, &new_mek.to_wire_bytes())
            .map_err(|e| MekRotationError::Crypto(format!("wrap MEK: {e}")))?;
        let payload = CommunityEnvelope::Control(ControlPayload::MekTransfer {
            community_id: community_id.to_string(),
            channel_id: channel_id.map(ToOwned::to_owned),
            generation,
            sender_pseudonym: my_pseudonym_hex.clone(),
            wrapped_mek: wrapped,
        });
        let bytes = rekindle_protocol::capnp_envelope::encode_community_envelope(&payload)
            .map_err(|e| MekRotationError::Transport(format!("encode MEK transfer: {e}")))?;

        let reply = deps
            .broadcast_to_peer(community_id, &recipient.pseudonym_hex, &recipient.route_blob, bytes)
            .await?;

        inspect_reply(community_id, &recipient.pseudonym_hex, channel_id, generation, &reply);
    }

    deps.emit_event(MekRotationEvent::RotationComplete {
        community_id: community_id.to_string(),
        channel_id: event_channel,
        generation,
    });

    Ok(())
}

/// P1.3 — verify the recipient sent back a structured
/// `MekTransferAck` confirming BOTH the network arrival and the
/// app-layer unwrap. Bare `ACK` reply means the recipient hit an
/// unwrap error (logged on their side); we record a debug trace so
/// the operator can investigate. Mismatches are warned but don't
/// hard-fail the rotation — the next rotation will recover.
fn inspect_reply(
    community_id: &str,
    recipient_hex: &str,
    expected_channel: Option<&str>,
    expected_generation: u64,
    reply: &[u8],
) {
    if reply == b"ACK" {
        tracing::debug!(
            community = %community_id,
            recipient = %recipient_hex,
            generation = expected_generation,
            "MEK transfer recipient replied with bare ACK (unwrap likely failed)"
        );
        return;
    }
    match rekindle_protocol::capnp_envelope::try_decode_community_envelope(reply) {
        Ok(Some(CommunityEnvelope::Control(ControlPayload::MekTransferAck {
            generation: ack_gen,
            channel_id: ack_channel,
            requester_pseudonym,
            ..
        }))) => {
            if ack_gen != expected_generation {
                tracing::warn!(
                    community = %community_id,
                    recipient = %recipient_hex,
                    sent_gen = expected_generation,
                    ack_gen,
                    "MekTransferAck generation mismatch"
                );
            } else if ack_channel.as_deref() != expected_channel {
                tracing::warn!(
                    community = %community_id,
                    recipient = %recipient_hex,
                    sent_channel = ?expected_channel,
                    ack_channel = ?ack_channel,
                    "MekTransferAck channel_id mismatch"
                );
            } else {
                tracing::trace!(
                    community = %community_id,
                    recipient = %recipient_hex,
                    requester = %requester_pseudonym,
                    generation = ack_gen,
                    "MekTransferAck verified"
                );
            }
        }
        Ok(_) => tracing::debug!(
            community = %community_id,
            recipient = %recipient_hex,
            "MEK transfer reply was not a MekTransferAck variant"
        ),
        Err(e) => tracing::debug!(
            community = %community_id,
            recipient = %recipient_hex,
            error = %e,
            "MEK transfer reply could not be decoded as community envelope"
        ),
    }
}

fn pseudonym_from_hex(hex_str: &str) -> Option<PseudonymKey> {
    let bytes = hex::decode(hex_str).ok()?;
    let array: [u8; 32] = bytes.try_into().ok()?;
    Some(PseudonymKey(array))
}
