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

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, MekTransferPayload};
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_crypto::group::mek_distribution::wrap_mek;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::PseudonymKey;

use crate::ack::{is_mek_transfer_ack, ExpectedAck};
use crate::pseudonym_hex::pseudonym_from_hex;

use crate::deps::{MekDistributeDeps, RotationRecipient};
use crate::election::cascade_delay;
use crate::error::MekRotationError;
use crate::event::MekRotationEvent;

/// Wait until it's our turn to attempt the rotation at `cascade_index`,
/// or yield if a peer already rotated.
///
/// `scope` is the key being rotated: the community key after a member
/// departure, a channel's key on a voice membership change.
///
/// Returns `Some(higher_priority_candidates)` if we should still
/// proceed (the slice we should NOT include in the recipient set when
/// distributing — they were the rotators before us, but they failed).
/// Returns `None` if we should not rotate (generation already advanced
/// or we aren't in the candidate list).
pub async fn wait_for_rotation_slot<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
    candidates: &[PseudonymKey],
    initial_generation: u64,
) -> Option<Vec<PseudonymKey>> {
    let me = deps.my_pseudonym(community_id)?;
    let index = candidates.iter().position(|candidate| candidate == &me)?;

    // Cascade levels >0 wait their backoff so the rightful primary mints
    // first. A session that ends during the wait does not rotate.
    if index > 0 {
        let token = deps.scope().token();
        token
            .run_until_cancelled(tokio::time::sleep(cascade_delay(index)))
            .await?;
    }

    // Final guard before claiming the slot — for EVERY index, INCLUDING the
    // primary (index 0), and for every scope. If another rotator already
    // advanced this scope's generation while we were electing/waiting,
    // stand down rather than mint a competing same-generation key. Two
    // peers that both believed they were the primary (divergent presence
    // views) would otherwise each mint at the same generation.
    if deps.cache().current_generation(community_id, scope) > initial_generation {
        return None;
    }

    Some(candidates.iter().take(index).cloned().collect())
}

/// Encrypt + dispatch the new MEK to each recipient. Emits
/// `RotationStarted` before the first send and `RotationComplete`
/// after the last one.
///
/// Returns the pseudonyms (hex) the push did not reach. A recipient whose
/// `app_call` fails is skipped, not fatal: the others still get the key,
/// and the unreached member repairs from the `MEKGenerationBump` by asking
/// for the key it lacks (plan E3.2 S7m item 1c, the D16 `RequestMEK`
/// path), as a missing ack is left to recovery. Each caller decides what
/// an unreached recipient means for it.
///
/// `recipients` should exclude our own pseudonym (the rotator); the
/// fn additionally filters out any recipient whose pseudonym matches
/// `my_pseudonym(community_id)` as a defensive measure.
pub async fn distribute_mek<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
    new_mek: &MediaEncryptionKey,
    recipients: &[RotationRecipient],
) -> Result<Vec<String>, MekRotationError> {
    let my_pseudonym = deps
        .my_pseudonym(community_id)
        .ok_or_else(|| MekRotationError::PseudonymMissing(community_id.to_string()))?;
    let my_pseudonym_hex = hex::encode(my_pseudonym.0);
    let generation = new_mek.generation();

    deps.emit_event(MekRotationEvent::RotationStarted {
        community_id: community_id.to_string(),
        scope,
        new_generation: generation,
        initiator_pseudonym_hex: my_pseudonym_hex.clone(),
    });

    let mut unreached = Vec::new();
    for recipient in recipients {
        // A rotation cut short by the session ending is repaired like one
        // nobody was online to receive: peers request the key.
        if deps.scope().is_closed() {
            break;
        }
        if recipient.pseudonym_hex == my_pseudonym_hex {
            // Defensive: should already have been excluded.
            continue;
        }
        let bytes =
            seal_mek_transfer(deps, community_id, scope, new_mek, &recipient.pseudonym_hex)?;

        let reply = match deps
            .broadcast_to_peer(
                community_id,
                &recipient.pseudonym_hex,
                &recipient.route_blob,
                bytes,
            )
            .await
        {
            Ok(reply) => reply,
            Err(error) => {
                tracing::info!(
                    community = %community_id,
                    recipient = %recipient.pseudonym_hex,
                    generation,
                    %error,
                    "key push did not reach the member; it asks for the key"
                );
                unreached.push(recipient.pseudonym_hex.clone());
                continue;
            }
        };

        // A missing ack is not a failure here (it is logged): the next
        // rotation recovers.
        let expected = ExpectedAck {
            community_id,
            inviter_hex: &my_pseudonym_hex,
            joiner_hex: &recipient.pseudonym_hex,
            scope,
            mek: new_mek,
        };
        if !is_mek_transfer_ack(&expected, &reply) {
            tracing::debug!(
                community = %community_id,
                recipient = %recipient.pseudonym_hex,
                generation,
                "rotation push not confirmed"
            );
        }
    }

    deps.emit_event(MekRotationEvent::RotationComplete {
        community_id: community_id.to_string(),
        scope,
        generation,
    });

    Ok(unreached)
}

/// A transfer id's length in bytes.
pub const TRANSFER_ID_LEN: usize = 16;

/// The encoded `MekTransfer` envelope of `mek` (of `scope`) sealed to
/// `recipient_hex`'s pseudonym under our community pseudonym key: the
/// payload `distribute_mek` calls each recipient with, and a
/// `mek_transfer` outbox row's (plan E3.2, D16), which never holds the
/// key itself.
///
/// # Errors
/// No identity, no pseudonym in `community_id`, an invalid recipient, or
/// the wrap or encoding failed.
pub fn seal_mek_transfer<D: MekDistributeDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
    mek: &MediaEncryptionKey,
    recipient_hex: &str,
) -> Result<Vec<u8>, MekRotationError> {
    let secret = deps
        .identity_secret()
        .ok_or(MekRotationError::IdentityNotLoaded)?;
    let my_signing_key =
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
    let my_pseudonym = deps
        .my_pseudonym(community_id)
        .ok_or_else(|| MekRotationError::PseudonymMissing(community_id.to_string()))?;
    let recipient = pseudonym_from_hex(recipient_hex).ok_or_else(|| {
        MekRotationError::InvalidInput(format!("invalid recipient pseudonym: {recipient_hex}"))
    })?;
    let wrapped = wrap_mek(&my_signing_key, &recipient.0, &mek.to_wire_bytes())
        .map_err(|e| MekRotationError::Crypto(format!("wrap MEK: {e}")))?;
    // Random per push; a retry resends these bytes, so it keeps its id.
    let mut transfer_id = vec![0u8; TRANSFER_ID_LEN];
    deps.rng().fill(&mut transfer_id);
    let payload = CommunityEnvelope::Control(ControlPayload::MekTransfer(MekTransferPayload {
        community_id: community_id.to_string(),
        channel_id: scope.wire_channel(),
        generation: mek.generation(),
        sender_pseudonym: hex::encode(my_pseudonym.0),
        wrapped_mek: wrapped,
        transfer_id,
    }));
    rekindle_codec::capnp_envelope::encode_community_envelope(&payload)
        .map_err(|e| MekRotationError::Transport(format!("encode MEK transfer: {e}")))
}
