//! Phase 13 — DM session lifecycle (architecture §27.1).
//!
//! `start_dm` (initiator path): allocate a 2-member SMPL record,
//! derive the deterministic ECDH MEK with the peer, persist the
//! conversation locally, watch the peer's subkey, and ship a
//! `DmInvite` via `app_call` (synchronous accept/decline reply).
//!
//! Parameterized over `DmDeps` so the crate stays free of `veilid-core`
//! and `AppState`. The adapter in `src-tauri/services/dm_adapter.rs`
//! handles the actual DHT record creation, subkey watch, and
//! Signal-encrypted `app_call` transport.

use rand::RngCore;
use rekindle_protocol::messaging::envelope::MessagePayload;
use rekindle_secrets::ed25519_dalek::SigningKey;

use crate::deps::DmDeps;
use crate::error::DmError;
use crate::invite::GroupDmParticipant;
use crate::mek::{derive_dm_mek, DmMek, DmMekChain};
use crate::store::DmInvitePending;

/// Allocate the SMPL record, derive the MEK, persist locally, send invite.
/// Returns the new record key on success. Errors map to `DmError` —
/// `InviteDeclined(record_key, reason)` would be the natural variant
/// for a peer-declined invite, but we map it to `InvalidInput` for now
/// to avoid expanding the error enum mid-phase; the wrapper in
/// src-tauri can re-shape if needed.
pub async fn start_dm<D: DmDeps + ?Sized>(
    deps: &D,
    bob_public_key_hex: &str,
    alice_pseudonym: &str,
) -> Result<String, DmError> {
    // 1. Identity material.
    let alice_secret = deps.identity_secret()?;
    let alice_identity = rekindle_crypto::Identity::from_secret_bytes(&alice_secret);
    let alice_ed_pub = alice_identity.public_key_bytes();
    let bob_ed_pub_bytes: [u8; 32] = hex::decode(bob_public_key_hex)
        .map_err(|e| DmError::InvalidInput(format!("invalid bob public key hex: {e}")))?
        .try_into()
        .map_err(|_| DmError::InvalidInput("bob public key must be 32 bytes".into()))?;

    // 2. Fresh slot seed (32 random bytes). SMPL plumbing only; never
    //    used to wrap content.
    let mut slot_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut slot_seed);
    let alice_slot_keypair = rekindle_secrets::derive::derive_slot_keypair(&slot_seed, 0)
        .map_err(|e| DmError::InvalidInput(format!("derive alice slot keypair: {e}")))?;
    let bob_slot_keypair = rekindle_secrets::derive::derive_slot_keypair(&slot_seed, 1)
        .map_err(|e| DmError::InvalidInput(format!("derive bob slot keypair: {e}")))?;
    let alice_slot_pub = alice_slot_keypair.verifying_key().to_bytes();
    let bob_slot_pub = bob_slot_keypair.verifying_key().to_bytes();

    // 3. Create the SMPL record with both members declared at o_cnt=0.
    //    The adapter constructs the veilid schema from the supplied
    //    member pubkeys.
    let record_key = deps
        .dht_create_smpl_record(vec![alice_slot_pub, bob_slot_pub])
        .await?;

    // 4. Derive the deterministic ECDH MEK and stash it locally.
    let alice_x25519_secret = alice_identity.to_x25519_secret();
    let bob_x25519_pub = rekindle_crypto::Identity::peer_ed25519_to_x25519(&bob_ed_pub_bytes)
        .map_err(|e| DmError::InvalidInput(format!("peer ed25519→x25519: {e}")))?;
    let mek = derive_dm_mek(
        alice_x25519_secret.as_bytes(),
        bob_x25519_pub.as_bytes(),
        &alice_ed_pub,
        &bob_ed_pub_bytes,
    )?;
    deps.mek_cache()
        .insert(&record_key, DmMekChain::new(mek));

    // 5. Persist the conversation. Alice owns subkey 0, Bob owns subkey 1.
    let participants = vec![
        GroupDmParticipant {
            pseudonym: alice_pseudonym.to_string(),
            subkey: 0,
            public_key: hex::encode(alice_ed_pub),
        },
        GroupDmParticipant {
            pseudonym: String::new(),
            subkey: 1,
            public_key: bob_public_key_hex.to_string(),
        },
    ];
    let owner_key = deps.owner_key()?;
    deps.store()
        .persist_invite_pending(
            &owner_key,
            DmInvitePending {
                record_key: record_key.clone(),
                is_group: false,
                initiator_public_key: hex::encode(alice_ed_pub),
                initiator_pseudonym: alice_pseudonym.to_string(),
                my_subkey: 0,
                participants,
                mek_generation: 0,
                slot_seed_hex: hex::encode(slot_seed),
                wrapped_mek_blob: None,
                created_at: i64::try_from(rekindle_utils::timestamp_ms() / 1000)
                    .unwrap_or(i64::MAX),
            },
        )
        .await?;

    // 6. Watch Bob's subkey so his replies arrive via the same
    //    DHT-watch pipeline community channels use (architecture §5.3
    //    line 1206).
    deps.dht_watch_subkeys(&record_key, vec![1]).await?;

    // 7. Ship the invite via `app_call` for an explicit
    //    DmAccept/DmDecline reply (architecture §27.1 line 2916). The
    //    MEK is *not* in the payload — Bob derives it from his identity
    //    + alice's public key.
    let invite = MessagePayload::DmInvite {
        record_key: record_key.clone(),
        slot_seed: slot_seed.to_vec(),
        alice_pseudonym: alice_pseudonym.to_string(),
        alice_subkey: 0,
        bob_subkey: 1,
    };
    let reply = deps.send_app_call(bob_public_key_hex, invite).await?;
    match reply {
        MessagePayload::DmAccept { record_key: rk } if rk == record_key => Ok(record_key),
        MessagePayload::DmDecline { record_key: rk, reason } if rk == record_key => {
            Err(DmError::InvalidInput(if reason.is_empty() {
                "DM invite declined".into()
            } else {
                format!("DM invite declined: {reason}")
            }))
        }
        other => Err(DmError::InvalidInput(format!(
            "unexpected DM invite reply: {other:?}"
        ))),
    }
}

/// Responder path (architecture §27.1 step 5): open the SMPL record
/// read-only, recover the MEK (re-derive for 2-party, unwrap for
/// group), restore the chain to the persisted generation, and watch
/// every peer subkey for inbound messages.
pub async fn accept_dm_invite<D: DmDeps + ?Sized>(
    deps: &D,
    record_key: &str,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;

    // 1. Load the persisted invite (initiator pubkey, my_subkey,
    //    mek_generation, is_group, wrapped MEK, participants).
    let meta = deps
        .store()
        .load_invite_meta(&owner_key, record_key)
        .await?
        .ok_or_else(|| DmError::SessionNotFound(record_key.to_string()))?;

    let initiator_ed_pub: [u8; 32] = hex::decode(&meta.initiator_public_key)
        .map_err(|e| DmError::InvalidInput(format!("invalid initiator pubkey hex: {e}")))?
        .try_into()
        .map_err(|_| DmError::InvalidInput("initiator pubkey must be 32 bytes".into()))?;

    // 2. Recover the MEK. 2-party DMs re-derive deterministically via
    //    ECDH; group DMs unwrap the per-recipient envelope from the
    //    initiator's `GroupDmInvite`.
    let bob_secret = deps.identity_secret()?;
    let bob_identity = rekindle_crypto::Identity::from_secret_bytes(&bob_secret);
    let mek = if meta.is_group {
        let wrapped = meta
            .wrapped_mek_blob
            .as_ref()
            .ok_or_else(|| DmError::InvalidInput("group DM has no wrapped MEK on accept".into()))?;
        let bob_signing = SigningKey::from_bytes(&bob_secret);
        let plain = rekindle_crypto::group::mek_distribution::unwrap_mek(
            &bob_signing,
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
        DmMek(key)
    } else {
        let initiator_x25519 =
            rekindle_crypto::Identity::peer_ed25519_to_x25519(&initiator_ed_pub)
                .map_err(|e| DmError::InvalidInput(format!("initiator ed25519→x25519: {e}")))?;
        let bob_ed_pub = bob_identity.public_key_bytes();
        derive_dm_mek(
            bob_identity.to_x25519_secret().as_bytes(),
            initiator_x25519.as_bytes(),
            &bob_ed_pub,
            &initiator_ed_pub,
        )?
    };

    // 3. Restore the chain to the persisted generation and stash it.
    let chain = DmMekChain::restore(mek, meta.mek_generation).map_err(|e| {
        DmError::InvalidSessionState(format!("dm chain restore: {e}"))
    })?;
    deps.mek_cache().insert(record_key, chain);

    // 4. Open read-only + watch all peer subkeys (everyone except us).
    deps.dht_open_record(record_key, None).await?;
    let peer_subkeys = peer_subkeys_for_watch(meta.is_group, meta.my_subkey, &meta.participants);
    if peer_subkeys.is_empty() {
        return Err(DmError::InvalidSessionState(
            "no peer subkeys to watch — invite shape is invalid".into(),
        ));
    }
    deps.dht_watch_subkeys(record_key, peer_subkeys).await?;
    Ok(())
}

/// Watch-set selection (architecture §27 watch lifecycle). 2-party DMs
/// always have exactly subkeys 0 + 1, so the peer's is the inverse of
/// ours. Group DMs include every participant slot except our own.
fn peer_subkeys_for_watch(
    is_group: bool,
    my_subkey: u32,
    participants: &[GroupDmParticipant],
) -> Vec<u32> {
    if !is_group {
        return vec![u32::from(my_subkey == 0)];
    }
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
    fn two_party_my_subkey_zero_watches_subkey_one() {
        assert_eq!(peer_subkeys_for_watch(false, 0, &[]), vec![1]);
    }

    #[test]
    fn two_party_my_subkey_one_watches_subkey_zero() {
        assert_eq!(peer_subkeys_for_watch(false, 1, &[]), vec![0]);
    }

    #[test]
    fn two_party_ignores_participants_list() {
        // Participants list is irrelevant for 2-party — the function
        // hard-codes peer = inverse of my_subkey.
        let participants = vec![p(5), p(99)];
        assert_eq!(peer_subkeys_for_watch(false, 0, &participants), vec![1]);
    }

    #[test]
    fn group_excludes_my_subkey() {
        let participants = vec![p(0), p(1), p(2), p(3)];
        let mut result = peer_subkeys_for_watch(true, 2, &participants);
        result.sort_unstable();
        assert_eq!(result, vec![0, 1, 3]);
    }

    #[test]
    fn group_with_only_my_slot_returns_empty() {
        // Edge case: malformed invite where the participant list only
        // has our own slot. accept_dm_invite errors on this; the
        // helper just returns empty.
        let participants = vec![p(0)];
        assert!(peer_subkeys_for_watch(true, 0, &participants).is_empty());
    }

    #[test]
    fn group_with_no_participants_returns_empty() {
        assert!(peer_subkeys_for_watch(true, 0, &[]).is_empty());
    }

    #[test]
    fn group_dedups_via_subkey_match() {
        // Defensive: if a participant somehow shares our subkey, it's
        // filtered out (we never watch our own subkey).
        let participants = vec![p(0), p(0), p(1)];
        assert_eq!(peer_subkeys_for_watch(true, 0, &participants), vec![1]);
    }
}
