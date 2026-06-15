//! DM session lifecycle: `start_dm` (initiator) + `accept_dm_invite` (responder).
//!
//! Dual-path invite: races `app_call` (sync, fast) against DHT inbox
//! write (durable, offline-safe). First success wins. Both paths write
//! the same `record_key` — receiver dedup via `ON CONFLICT DO NOTHING`
//! on `dm_persist_invite`.

use std::time::Duration;

use rekindle_types::dm_store::{DmInvitePending, DmParticipant};

use aws_lc_rs::rand::SecureRandom;

use crate::dm::deps::DmDeps;
use crate::dm::error::DmError;
use crate::dm::invite::DmInvite;
use crate::dm::mek_chain::{derive_dm_mek, DmMekChain};

const APP_CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Initiator: create SMPL record, persist, watch, invite via dual path.
///
/// 1. Generate random slot_seed, derive both slot keypairs
/// 2. Create 2-member SMPL record
/// 3. For 1:1: PQXDH session already established via friendship flow
///    (the peer is a friend — session exists in SessionCache)
///    For group: derive deterministic ECDH MEK
/// 4. Persist conversation locally
/// 5. Watch peer's subkey
/// 6. Race: app_call (fast) + DHT inbox (durable)
pub async fn start_dm(
    deps: &dyn DmDeps,
    peer_public_key_hex: &str,
    my_pseudonym: &str,
    is_group: bool,
) -> Result<String, DmError> {
    let my_pub_bytes = deps.identity_public_key_bytes()?;
    let my_pub_hex = deps.identity_public_key_hex()?;

    let peer_pub_bytes: [u8; 32] = hex::decode(peer_public_key_hex)
        .map_err(|e| DmError::InvalidInput(format!("peer pubkey hex: {e}")))?
        .try_into()
        .map_err(|_| DmError::InvalidInput("peer pubkey must be 32 bytes".into()))?;

    // ── Slot seed + keypairs ────────────────────────────────────
    let mut slot_seed = [0u8; 32];
    aws_lc_rs::rand::SystemRandom::new()
        .fill(&mut slot_seed)
        .map_err(|e| DmError::EncryptFailed(format!("slot seed rng: {e}")))?;

    let my_slot_kp = rekindle_identity::derive_slot_keypair(&slot_seed, 0)
        .map_err(|e| DmError::InvalidInput(format!("derive slot 0: {e}")))?;
    let peer_slot_kp = rekindle_identity::derive_slot_keypair(&slot_seed, 1)
        .map_err(|e| DmError::InvalidInput(format!("derive slot 1: {e}")))?;

    // ── Create SMPL record ──────────────────────────────────────
    let record_key = deps
        .dht_create_smpl_record(vec![
            my_slot_kp.public_key_bytes(),
            peer_slot_kp.public_key_bytes(),
        ])
        .await?;

    tracing::info!(
        record_key = &record_key[..20.min(record_key.len())],
        peer = &peer_public_key_hex[..16.min(peer_public_key_hex.len())],
        is_group,
        "dm::session: SMPL record created"
    );

    // ── Group MEK (if group) ────────────────────────────────────
    if is_group {
        let my_x25519 = deps.x25519_identity_seed()?;
        // Read the peer's X25519 DH public key from their profile DHT.
        // Cannot derive from Ed25519 pub — birational map prohibited (R-06).
        let peer_x25519_pub = deps
            .read_peer_x25519_pub(peer_public_key_hex)
            .await?;
        let mek = derive_dm_mek(
            &my_x25519,
            &peer_x25519_pub,
            &my_pub_bytes,
            &peer_pub_bytes,
        )?;
        deps.mek_cache().insert(&record_key, DmMekChain::new(mek));
        tracing::debug!(
            record_key = &record_key[..12.min(record_key.len())],
            "dm::session: group MEK derived and cached"
        );
    }
    // 1:1: no MEK needed — Triple Ratchet session already exists from friendship

    // ── Persist conversation ────────────────────────────────────
    let participants = vec![
        DmParticipant {
            pseudonym: my_pseudonym.into(),
            subkey: 0,
            public_key: my_pub_hex.clone(),
        },
        DmParticipant {
            pseudonym: String::new(),
            subkey: 1,
            public_key: peer_public_key_hex.into(),
        },
    ];

    deps.store().dm_persist_invite(DmInvitePending {
        record_key: record_key.clone(),
        is_group,
        initiator_public_key: my_pub_hex,
        initiator_pseudonym: my_pseudonym.into(),
        my_subkey: 0,
        participants,
        mek_generation: 0,
        slot_seed_hex: hex::encode(slot_seed),
        wrapped_mek_blob: None,
    })?;

    // ── Watch peer's subkey ─────────────────────────────────────
    deps.dht_watch_subkeys(&record_key, vec![1]).await?;

    // ── Dual-path invite ────────────────────────────────────────
    let invite_payload = serde_json::to_vec(&DmInvite {
        record_key: record_key.clone(),
        slot_seed: slot_seed.to_vec(),
        initiator_pseudonym: my_pseudonym.into(),
        initiator_subkey: 0,
        responder_subkey: 1,
    })
    .map_err(|e| DmError::EncryptFailed(format!("invite serialize: {e}")))?;

    // Path A: app_call (fast, requires peer online)
    let app_call_result = deps
        .send_app_call(peer_public_key_hex, &invite_payload, APP_CALL_TIMEOUT)
        .await;

    // Path B: DHT inbox (durable, always — backup even if app_call succeeded)
    if let Err(e) = deps
        .write_dm_invite_to_inbox(peer_public_key_hex, &invite_payload)
        .await
    {
        tracing::warn!(
            error = %e,
            "dm::session: DHT inbox write failed — app_call is sole path"
        );
    }

    match app_call_result {
        Ok(reply) => {
            if let Ok(accept) = serde_json::from_slice::<DmAcceptReply>(&reply) {
                if accept.accepted {
                    tracing::info!(
                        record_key = &record_key[..12.min(record_key.len())],
                        "dm::session: invite accepted via app_call (fast path)"
                    );
                } else {
                    tracing::info!(
                        record_key = &record_key[..12.min(record_key.len())],
                        reason = %accept.reason,
                        "dm::session: invite declined via app_call"
                    );
                    return Err(DmError::InvalidInput(format!(
                        "DM invite declined: {}",
                        accept.reason
                    )));
                }
            }
        }
        Err(e) => {
            tracing::debug!(
                error = %e,
                "dm::session: app_call failed — DHT inbox is sole path"
            );
        }
    }

    Ok(record_key)
}

#[derive(serde::Deserialize)]
struct DmAcceptReply {
    accepted: bool,
    #[serde(default)]
    reason: String,
}

/// Responder: open SMPL record, recover encryption material, watch peers.
///
/// For 1:1: Triple Ratchet session already established via friendship flow.
/// For group: derive MEK from ECDH, restore chain to persisted generation.
pub async fn accept_dm_invite(
    deps: &dyn DmDeps,
    record_key: &str,
) -> Result<(), DmError> {
    let meta = deps
        .store()
        .dm_load_invite_meta(record_key)?
        .ok_or_else(|| DmError::SessionNotFound(record_key.into()))?;

    tracing::info!(
        record_key = &record_key[..20.min(record_key.len())],
        is_group = meta.is_group,
        my_subkey = meta.my_subkey,
        "dm::session: accepting invite"
    );

    // ── Group: recover MEK ──────────────────────────────────────
    if meta.is_group {
        let initiator_pub: [u8; 32] = hex::decode(&meta.initiator_public_key)
            .map_err(|e| DmError::InvalidInput(format!("initiator pubkey hex: {e}")))?
            .try_into()
            .map_err(|_| DmError::InvalidInput("initiator pubkey must be 32 bytes".into()))?;

        let my_pub = deps.identity_public_key_bytes()?;
        let my_x25519 = deps.x25519_identity_seed()?;
        // Read the initiator's X25519 DH public key from their profile DHT.
        // Cannot derive from Ed25519 pub — birational map prohibited (R-06).
        let initiator_x25519_pub = deps
            .read_peer_x25519_pub(&meta.initiator_public_key)
            .await?;

        let mek = derive_dm_mek(
            &my_x25519,
            &initiator_x25519_pub,
            &my_pub,
            &initiator_pub,
        )?;

        let chain = DmMekChain::restore(mek, meta.mek_generation)?;
        deps.mek_cache().insert(record_key, chain);

        tracing::debug!(
            record_key = &record_key[..12.min(record_key.len())],
            mek_generation = meta.mek_generation,
            "dm::session: group MEK chain restored"
        );
    }
    // 1:1: no MEK setup needed — Triple Ratchet session exists from friendship

    // ── Open record + watch peer subkeys ────────────────────────
    deps.dht_open_record(record_key).await?;

    let peer_subkeys = peer_subkeys_for_watch(meta.is_group, meta.my_subkey, &meta.participants);
    if peer_subkeys.is_empty() {
        return Err(DmError::InvalidSessionState(
            "no peer subkeys to watch — invite shape invalid".into(),
        ));
    }

    deps.dht_watch_subkeys(record_key, peer_subkeys.clone())
        .await?;

    tracing::info!(
        record_key = &record_key[..12.min(record_key.len())],
        watching = ?peer_subkeys,
        "dm::session: invite accepted, watches established"
    );

    Ok(())
}

/// Watch-set selection. 2-party DMs always have subkeys 0 + 1, so the
/// peer's is the inverse of ours. Group DMs include every participant
/// slot except our own.
fn peer_subkeys_for_watch(
    is_group: bool,
    my_subkey: u32,
    participants: &[DmParticipant],
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

    fn p(subkey: u32) -> DmParticipant {
        DmParticipant {
            pseudonym: format!("p{subkey}"),
            subkey,
            public_key: format!("pk{subkey}"),
        }
    }

    #[test]
    fn two_party_my_zero_watches_one() {
        assert_eq!(peer_subkeys_for_watch(false, 0, &[]), vec![1]);
    }

    #[test]
    fn two_party_my_one_watches_zero() {
        assert_eq!(peer_subkeys_for_watch(false, 1, &[]), vec![0]);
    }

    #[test]
    fn two_party_ignores_participants() {
        let ps = vec![p(5), p(99)];
        assert_eq!(peer_subkeys_for_watch(false, 0, &ps), vec![1]);
    }

    #[test]
    fn group_excludes_my_subkey() {
        let ps = vec![p(0), p(1), p(2), p(3)];
        let mut result = peer_subkeys_for_watch(true, 2, &ps);
        result.sort_unstable();
        assert_eq!(result, vec![0, 1, 3]);
    }

    #[test]
    fn group_only_my_slot_returns_empty() {
        let ps = vec![p(0)];
        assert!(peer_subkeys_for_watch(true, 0, &ps).is_empty());
    }

    #[test]
    fn group_no_participants_returns_empty() {
        assert!(peer_subkeys_for_watch(true, 0, &[]).is_empty());
    }
}
