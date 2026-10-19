//! Phase 13 — group DM session lifecycle (architecture §27.2).
//!
//! `accept_dm_invite` (responder path): unwrap the group MEK the
//! initiator wrapped for us, restore the chain, and watch every other
//! member's subkey. The 1:1 SMPL path (`start_dm`, the ECDH MEK) was
//! deleted by plan E2.3: 1:1 DMs are the friend's ratchet stream.
//!
//! Parameterized over `DmDeps` so the crate stays free of `veilid-core`
//! and `AppState`. The adapter in `src-tauri/services/dm_adapter.rs`
//! handles the actual DHT record access and subkey watch.

use rekindle_secrets::ed25519_dalek::SigningKey;

use crate::deps::DmDeps;
use crate::error::DmError;
use crate::invite::GroupDmParticipant;
use crate::mek::{DmMek, DmMekChain};

/// Responder path (architecture §27.2): unwrap the per-recipient group
/// MEK from the initiator's `GroupDmInvite`, restore the chain to the
/// persisted generation, borrow the record and watch every other
/// member's subkey for inbound messages.
pub async fn accept_dm_invite<D: DmDeps + ?Sized>(
    deps: &D,
    record_key: &str,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;

    // 1. Load the persisted invite (initiator pubkey, my_subkey,
    //    mek_generation, wrapped MEK, participants).
    let meta = deps
        .store()
        .load_invite_meta(&owner_key, record_key)
        .await?
        .ok_or_else(|| DmError::SessionNotFound(record_key.to_string()))?;

    let initiator_ed_pub: [u8; 32] = hex::decode(&meta.initiator_public_key)
        .map_err(|e| DmError::InvalidInput(format!("invalid initiator pubkey hex: {e}")))?
        .try_into()
        .map_err(|_| DmError::InvalidInput("initiator pubkey must be 32 bytes".into()))?;

    // 2. Unwrap the per-recipient envelope from the `GroupDmInvite`.
    let wrapped = meta
        .wrapped_mek_blob
        .as_ref()
        .ok_or_else(|| DmError::InvalidInput("group DM has no wrapped MEK on accept".into()))?;
    let my_secret = deps.identity_secret()?;
    let my_signing = SigningKey::from_bytes(&my_secret);
    let plain = rekindle_crypto::group::mek_distribution::unwrap_mek(
        &my_signing,
        &initiator_ed_pub,
        wrapped,
    )
    .map_err(|e| DmError::DecryptFailed(format!("unwrap group mek: {e}")))?;
    // Wire format: 8-byte LE generation + 32-byte key.
    if plain.len() != 40 {
        return Err(DmError::DecryptFailed(format!(
            "group MEK plaintext wrong length: {}",
            plain.len()
        )));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&plain[8..]);

    // 3. Restore the chain to the persisted generation and stash it.
    let chain = DmMekChain::restore(DmMek(key), meta.mek_generation)
        .map_err(|e| DmError::InvalidSessionState(format!("dm chain restore: {e}")))?;
    deps.mek_cache().insert(record_key, chain);

    // 4. Borrow the record and watch all other members' subkeys; the host
    //    keeps the lease for the session.
    let peer_subkeys = peer_subkeys_for_watch(meta.my_subkey, &meta.participants);
    if peer_subkeys.is_empty() {
        return Err(DmError::InvalidSessionState(
            "no peer subkeys to watch — invite shape is invalid".into(),
        ));
    }
    let lease = deps.dht_acquire_record(record_key, None).await?;
    if let Err(e) = deps.dht_watch_subkeys(lease, peer_subkeys).await {
        deps.dht_release_record(lease).await;
        return Err(e);
    }
    deps.dht_hold_session(record_key, lease).await;
    Ok(())
}

/// Watch-set selection (architecture §27 watch lifecycle): every
/// participant slot except our own.
fn peer_subkeys_for_watch(my_subkey: u32, participants: &[GroupDmParticipant]) -> Vec<u32> {
    participants
        .iter()
        .filter(|p| p.subkey != my_subkey)
        .map(|p| p.subkey)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(subkey: u32) -> GroupDmParticipant {
        GroupDmParticipant {
            pseudonym: format!("p{subkey}"),
            subkey,
            public_key: format!("pk{subkey}"),
        }
    }

    #[test]
    fn excludes_my_subkey() {
        let participants = vec![p(0), p(1), p(2), p(3)];
        let mut result = peer_subkeys_for_watch(2, &participants);
        result.sort_unstable();
        assert_eq!(result, vec![0, 1, 3]);
    }

    #[test]
    fn only_my_slot_returns_empty() {
        // Edge case: malformed invite where the participant list only
        // has our own slot. accept_dm_invite errors on this; the
        // helper just returns empty.
        let participants = vec![p(0)];
        assert!(peer_subkeys_for_watch(0, &participants).is_empty());
    }

    #[test]
    fn no_participants_returns_empty() {
        assert!(peer_subkeys_for_watch(0, &[]).is_empty());
    }

    #[test]
    fn dedups_via_subkey_match() {
        // Defensive: if a participant somehow shares our subkey, it's
        // filtered out (we never watch our own subkey).
        let participants = vec![p(0), p(0), p(1)];
        assert_eq!(peer_subkeys_for_watch(0, &participants), vec![1]);
    }
}
