//! Outbound DM creation (architecture §27.1 — 2-party).
//!
//! Alice creates a 2-member SMPL record (`o_cnt: 0`), derives the
//! deterministic ECDH MEK with Bob, persists the conversation locally,
//! and sends `DmInvite` so Bob can derive the same MEK and open the
//! record. The MEK is *never* transmitted — both peers derive it
//! independently from their identity keys.

use std::sync::Arc;

use rand::RngCore;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_dm::{derive_dm_mek, GroupDmParticipant};
use rekindle_protocol::messaging::envelope::MessagePayload;
use rekindle_records::schema;
use veilid_core::{ValueSubkeyRangeSet, CRYPTO_KIND_VLD0};

use crate::db::DbPool;
use crate::services::message_service;
use crate::state::AppState;
use crate::state_helpers;

/// Allocate the SMPL record, derive the MEK, persist locally, send invite.
/// Returns the new record key on success.
pub async fn start_dm(
    state: &Arc<AppState>,
    pool: &DbPool,
    bob_public_key_hex: &str,
    alice_pseudonym: &str,
) -> Result<String, String> {
    // 1. Identity material.
    let alice_secret = {
        let s = state.identity_secret.lock();
        *s.as_ref().ok_or("identity not unlocked")?
    };
    let alice_identity = rekindle_crypto::Identity::from_secret_bytes(&alice_secret);
    let alice_ed_pub = alice_identity.public_key_bytes();
    let bob_ed_pub_bytes: [u8; 32] = hex::decode(bob_public_key_hex)
        .map_err(|e| format!("invalid bob public key hex: {e}"))?
        .try_into()
        .map_err(|_| "bob public key must be 32 bytes".to_string())?;

    // 2. Generate a fresh slot seed (32 random bytes) — used for SMPL
    //    member-slot keypair derivation. Distinct from the MEK; this is
    //    Veilid plumbing only, never used to wrap content.
    let mut slot_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut slot_seed);
    let alice_slot_keypair: SigningKey =
        rekindle_secrets::derive::derive_slot_keypair(&slot_seed, 0)
            .map_err(|e| format!("derive alice slot keypair: {e}"))?;
    let bob_slot_keypair: SigningKey = rekindle_secrets::derive::derive_slot_keypair(&slot_seed, 1)
        .map_err(|e| format!("derive bob slot keypair: {e}"))?;
    let alice_slot_pub = alice_slot_keypair.verifying_key().to_bytes();
    let bob_slot_pub = bob_slot_keypair.verifying_key().to_bytes();

    // 3. Create the SMPL record with both members declared at o_cnt=0.
    let rc = state_helpers::safe_routing_context(state)
        .ok_or_else(|| "no routing context".to_string())?;
    let smpl_schema = schema::community_smpl_schema(&[alice_slot_pub, bob_slot_pub])
        .map_err(|e| format!("dm schema: {e}"))?;
    let descriptor = rc
        .create_dht_record(CRYPTO_KIND_VLD0, smpl_schema, None)
        .await
        .map_err(|e| format!("create dm dht record: {e}"))?;
    let record_key = descriptor.key().to_string();

    // 4. Derive the deterministic ECDH MEK and stash it locally so we
    //    can encrypt outbound messages and decrypt Bob's incoming ones.
    let alice_x25519_secret = alice_identity.to_x25519_secret();
    let bob_x25519_pub = rekindle_crypto::Identity::peer_ed25519_to_x25519(&bob_ed_pub_bytes)
        .map_err(|e| format!("peer ed25519→x25519: {e}"))?;
    let mek = derive_dm_mek(
        alice_x25519_secret.as_bytes(),
        bob_x25519_pub.as_bytes(),
        &alice_ed_pub,
        &bob_ed_pub_bytes,
    )
    .map_err(|e| format!("derive dm mek: {e}"))?;
    state
        .dm_mek_cache
        .lock()
        .insert(record_key.clone(), rekindle_dm::DmMekChain::new(mek));

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
    super::store::persist_dm_invite_pending(
        state,
        pool,
        &record_key,
        false,
        &hex::encode(alice_ed_pub),
        alice_pseudonym,
        0,
        &participants,
        0,
        &hex::encode(slot_seed),
        None,
    )
    .await?;

    // 6. Watch Bob's subkey so his replies arrive via the same DHT-watch
    //    pipeline community channels use (architecture §5.3 line 1206 —
    //    every participant opens + watches the SMPL record). Without this,
    //    Alice would only learn of Bob's writes via the 60s inspect loop.
    let _ = rc
        .watch_dht_values(
            descriptor.key().clone(),
            Some(ValueSubkeyRangeSet::single(1)),
            None,
            None,
        )
        .await
        .map_err(|e| format!("watch dm subkey 1: {e}"))?;

    // 7. Ship the invite via `app_call` so we get an explicit
    //    DmAccept/DmDecline reply (architecture §27.1 line 2916). The
    //    MEK is *not* in the payload — Bob derives it.
    let payload = MessagePayload::DmInvite {
        record_key: record_key.clone(),
        slot_seed: slot_seed.to_vec(),
        alice_pseudonym: alice_pseudonym.to_string(),
        alice_subkey: 0,
        bob_subkey: 1,
    };
    let reply = message_service::send_to_peer_call(state, bob_public_key_hex, &payload).await?;
    match reply {
        MessagePayload::DmAccept { record_key: rk } if rk == record_key => Ok(record_key),
        MessagePayload::DmDecline { record_key: rk, reason } if rk == record_key => {
            Err(if reason.is_empty() {
                "DM invite declined".to_string()
            } else {
                format!("DM invite declined: {reason}")
            })
        }
        other => Err(format!("unexpected DM invite reply: {other:?}")),
    }
}
