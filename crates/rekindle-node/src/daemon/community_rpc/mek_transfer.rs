//! Inbound wrapped-MEK delivery from the deterministic rotator.
//!
//! `communities-channels.md` ("Distribution paths") puts this on the
//! critical path for forward secrecy: when a member departs, the
//! elected rotator generates a new key and *delivers the wrapped MEK to
//! each member via `app_call`*. Without this handler the daemon has no
//! way to receive one — it learned MEKs only from a `JoinAccepted`
//! gossip payload and from the v1.0 registry MEK vault, and the vault
//! is being retired. A daemon member would have gone silently
//! undecryptable after the first rotation in any community it shares
//! with a desktop peer.
//!
//! The envelope arrives unsigned (see
//! `Caller::call_community_envelope`); authenticity comes from the
//! wrap itself, because `unwrap_mek` feeds the rotator's pseudonym
//! public key into the ECDH. A forged `sender_pseudonym` therefore
//! fails to decrypt rather than being believed — which is why the only
//! thing this handler does with the claimed sender is try it as a key.

use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_protocol::dht::community::envelope::MekTransferPayload;
use rekindle_transport::payload::rpc::CallResponse;

use crate::daemon::governance_adapter::COMMUNITY_MEK_SLOT;

/// Accept a wrapped MEK and cache it.
///
/// Returns `CallResponse::Ok(pseudonym_hex)` so the transport layer can
/// name us in the `MekTransferAck` the rotator is waiting on; anything
/// else makes the rotator treat the delivery as failed, which is the
/// correct signal when we could not unwrap.
pub(crate) fn handle_mek_transfer(
    transfer: &MekTransferPayload,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &Arc<RwLock<rekindle_transport::crypto::mek::MekCache>>,
) -> CallResponse {
    let governance_key = &transfer.community_id;

    // Membership is checked first so an unsolicited transfer for a
    // community we are not in is refused before any crypto runs.
    let is_member = session
        .read()
        .as_ref()
        .is_some_and(|s| s.community(governance_key).is_some());
    if !is_member {
        tracing::debug!(
            community = %&governance_key[..16.min(governance_key.len())],
            "MEK transfer for a community we are not in — refusing"
        );
        return CallResponse::Rejected {
            reason: "not a member".into(),
        };
    }

    let Some(signing_key_bytes) = super::get_signing_key(signing_key) else {
        tracing::warn!("daemon locked — cannot unwrap MEK transfer");
        return CallResponse::Rejected {
            reason: "locked".into(),
        };
    };

    // `channel_id: None` is the community-wide key. It maps to the same
    // reserved slot the governance adapter reads from, so a rotated
    // community MEK lands where `community_mek` looks for it rather
    // than under an empty channel id nothing queries.
    let channel_id = transfer
        .channel_id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| COMMUNITY_MEK_SLOT.to_string());

    let projected = rekindle_transport::payload::rpc::MekTransferPayload {
        channel_id,
        generation: transfer.generation,
        rotator_pseudonym_hex: transfer.sender_pseudonym.clone(),
        wrapped_mek: transfer.wrapped_mek.clone(),
    };

    match rekindle_transport::operations::mek::receive_mek_transfer_payload(
        &projected,
        &signing_key_bytes,
        governance_key,
        mek_cache,
    ) {
        Ok(generation) => {
            let our_pseudonym = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(
                &signing_key_bytes,
                governance_key,
            );
            let our_pseudonym_hex = hex::encode(our_pseudonym.verifying_key().to_bytes());
            tracing::info!(
                community = %&governance_key[..16.min(governance_key.len())],
                channel = ?transfer.channel_id,
                generation,
                "MEK accepted from rotator"
            );
            CallResponse::Ok(our_pseudonym_hex.into_bytes())
        }
        Err(e) => {
            // Expected whenever the transfer was not actually wrapped
            // for us — a misrouted delivery or a forged sender. Logged
            // at warn because a *legitimate* rotator hitting this means
            // a real key-distribution failure.
            tracing::warn!(
                community = %&governance_key[..16.min(governance_key.len())],
                error = %e,
                "MEK transfer unwrap failed"
            );
            CallResponse::Rejected {
                reason: "unwrap failed".into(),
            }
        }
    }
}
