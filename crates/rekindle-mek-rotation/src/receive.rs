//! Phase 23.D.10 — MEK receive path ported from
//! `src-tauri/services/community/mek_rotation.rs`. Unwraps a sealed
//! `MekTransfer` payload via the recipient's identity-derived
//! pseudonym key, writes it into the cache, persists to keystore, and
//! emits the rotation event. The wrap math + cascade election remain
//! in `distribute.rs`.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_crypto::group::mek_distribution::unwrap_mek;
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

use rekindle_types::channel_keys::KeyScope;

use crate::deps::{ChannelMekCache, MekDistributeDeps};
use crate::error::MekRotationError;

pub fn unwrap_received_mek(
    community_id: &str,
    recipient_secret: &[u8; 32],
    sender_pseudonym: &str,
    wrapped_mek: &[u8],
) -> Result<MediaEncryptionKey, MekRotationError> {
    let my_signing_key = derive_community_pseudonym(recipient_secret, community_id);
    let sender_bytes = hex::decode(sender_pseudonym).map_err(|e| {
        MekRotationError::InvalidInput(format!("invalid sender pseudonym hex: {e}"))
    })?;
    let sender_pub: [u8; 32] = sender_bytes.try_into().map_err(|_| {
        MekRotationError::InvalidInput("sender pseudonym must be 32 bytes".to_string())
    })?;
    let mek_wire = unwrap_mek(&my_signing_key, &sender_pub, wrapped_mek)
        .map_err(|e| MekRotationError::InvalidInput(format!("unwrap MEK failed: {e}")))?;
    MediaEncryptionKey::from_wire_bytes(&mek_wire)
        .ok_or_else(|| MekRotationError::InvalidInput("invalid MEK wire bytes".to_string()))
}

/// True iff the cache holds `scope`'s key at exactly `generation`.
/// Used by the requester-side retry loop to stop once the `MekTransfer`
/// it asked for has landed. Exact scope only: a community key never
/// satisfies a request for a channel's key (RFC 9605 §4.4.1 — a key is
/// selected by its id, never by substitution).
#[must_use]
pub fn mek_cache_has_generation(
    cache: &dyn ChannelMekCache,
    community_id: &str,
    scope: KeyScope,
    generation: u64,
) -> bool {
    cache.get(community_id, scope, generation).is_some()
}

pub fn handle_incoming_mek_transfer<D: MekDistributeDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
    sender_pseudonym: &str,
    wrapped_mek: &[u8],
) -> Result<u64, MekRotationError> {
    let secret = deps
        .identity_secret()
        .ok_or_else(|| MekRotationError::InvalidInput("identity secret unavailable".to_string()))?;
    let mek = unwrap_received_mek(community_id, &secret, sender_pseudonym, wrapped_mek)?;
    let generation = mek.generation();
    if deps.apply_received_mek_to_state(community_id, scope, &mek) {
        deps.persist_received_mek(community_id, scope, &mek);
        deps.emit_rotation_received(community_id, scope, generation);
    }
    Ok(generation)
}
