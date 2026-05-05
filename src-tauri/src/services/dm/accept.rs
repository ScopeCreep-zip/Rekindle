//! Bob's accept path (architecture §27.1 step 5):
//! open the SMPL record read-only, derive the deterministic MEK from
//! identity keys, watch the initiator's subkey for incoming messages.

use std::sync::Arc;

use rekindle_dm::{derive_dm_mek, DmMek, DmMekChain, GroupDmParticipant};
use rekindle_secrets::ed25519_dalek::SigningKey;
use veilid_core::{RecordKey, ValueSubkeyRangeSet};

use crate::db::DbPool;
use crate::db_helpers::db_call_or_default;
use crate::state::AppState;
use crate::state_helpers;

pub async fn accept_dm_invite(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }

    // 1. Look up the persisted invite. For group DMs we also need the
    //    wrapped MEK blob and the participant list (for selecting which
    //    subkeys to watch); for 2-party DMs the MEK is re-derived.
    let row: Option<(String, i64, i64, bool, Option<Vec<u8>>, String)> = {
        let owner = owner_key.clone();
        let record = record_key.to_string();
        db_call_or_default(pool, move |conn| {
            let r = conn
                .query_row(
                    "SELECT initiator_public_key, my_subkey, mek_generation, is_group,
                            wrapped_mek_blob, participants_json
                     FROM dms WHERE owner_key = ?1 AND record_key = ?2",
                    rusqlite::params![owner, record],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)? != 0,
                            row.get::<_, Option<Vec<u8>>>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    },
                )
                .ok();
            Ok(r)
        })
        .await
    };
    let (
        initiator_pubkey_hex,
        my_subkey_i,
        mek_generation_i,
        is_group,
        wrapped_mek_blob,
        participants_json,
    ) = row.ok_or("dm row not found — was the invite persisted?")?;
    let my_subkey = u32::try_from(my_subkey_i).unwrap_or(1);
    let mek_generation = u64::try_from(mek_generation_i).unwrap_or(0);

    // 2. Recover the MEK. 2-party DMs re-derive deterministically via
    //    ECDH (architecture §27.1). Group DMs unwrap the wrapped envelope
    //    that was sent via `GroupDmInvite` (architecture §27.2).
    let initiator_ed_pub: [u8; 32] = hex::decode(&initiator_pubkey_hex)
        .map_err(|e| format!("invalid initiator pubkey hex: {e}"))?
        .try_into()
        .map_err(|_| "initiator pubkey must be 32 bytes".to_string())?;
    let bob_secret = {
        let s = state.identity_secret.lock();
        *s.as_ref().ok_or("identity not unlocked")?
    };
    let bob_identity = rekindle_crypto::Identity::from_secret_bytes(&bob_secret);

    let mek = if is_group {
        let wrapped = wrapped_mek_blob
            .ok_or_else(|| "group DM has no wrapped MEK on accept".to_string())?;
        let bob_signing = SigningKey::from_bytes(&bob_secret);
        let plain = rekindle_crypto::group::mek_distribution::unwrap_mek(
            &bob_signing,
            &initiator_ed_pub,
            &wrapped,
        )
        .map_err(|e| format!("unwrap group mek: {e}"))?;
        // wire format: 8-byte LE generation + 32-byte key.
        if plain.len() != 40 {
            return Err(format!("group MEK plaintext wrong length: {}", plain.len()));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&plain[8..]);
        DmMek(key)
    } else {
        let initiator_x25519 =
            rekindle_crypto::Identity::peer_ed25519_to_x25519(&initiator_ed_pub)
                .map_err(|e| format!("initiator ed25519→x25519: {e}"))?;
        let bob_ed_pub = bob_identity.public_key_bytes();
        derive_dm_mek(
            bob_identity.to_x25519_secret().as_bytes(),
            initiator_x25519.as_bytes(),
            &bob_ed_pub,
            &initiator_ed_pub,
        )
        .map_err(|e| format!("derive dm mek: {e}"))?
    };
    // Build a chain rooted at the genesis MEK and seek to the persisted
    // generation. The genesis MEK is what we just derived; for 2-party
    // DMs that's the ECDH-derived gen-0 MEK, and for group DMs it's the
    // unwrapped MEK that the initiator generated. In both cases the
    // persisted `mek_generation` is the chain's tip.
    let chain = DmMekChain::restore(mek, mek_generation)
        .map_err(|e| format!("dm chain restore: {e}"))?;
    state
        .dm_mek_cache
        .lock()
        .insert(record_key.to_string(), chain);

    // 3. Open the SMPL record read-only and watch every participant
    //    subkey EXCEPT our own. For 2-party DMs that's just 1 subkey
    //    (the other peer); for group DMs that's N-1 subkeys.
    let rc = state_helpers::safe_routing_context(state)
        .ok_or_else(|| "no routing context".to_string())?;
    let record_key_typed = record_key
        .parse::<RecordKey>()
        .map_err(|e| format!("invalid record key: {e}"))?;
    let _ = rc
        .open_dht_record(record_key_typed.clone(), None)
        .await
        .map_err(|e| format!("open dm record: {e}"))?;

    let watch_set = build_peer_watch_set(is_group, my_subkey, &participants_json);
    if watch_set.is_empty() {
        return Err("no peer subkeys to watch — invite shape is invalid".into());
    }
    let _ = rc
        .watch_dht_values(record_key_typed, Some(watch_set), None, None)
        .await
        .map_err(|e| format!("watch dm subkeys: {e}"))?;

    Ok(())
}

/// Architecture §27 watch lifecycle: every participant watches the
/// other participants' subkeys after open. Returns the union of all
/// peer subkey indices, omitting our own.
fn build_peer_watch_set(
    is_group: bool,
    my_subkey: u32,
    participants_json: &str,
) -> ValueSubkeyRangeSet {
    if !is_group {
        // 2-party DMs always have exactly two subkeys (0 and 1); the
        // peer's is the inverse of ours. No need to parse JSON.
        let peer = u32::from(my_subkey == 0);
        return ValueSubkeyRangeSet::single(peer);
    }
    let participants: Vec<GroupDmParticipant> =
        serde_json::from_str(participants_json).unwrap_or_default();
    let mut out = ValueSubkeyRangeSet::new();
    for participant in participants {
        if participant.subkey == my_subkey {
            continue;
        }
        out = out.union(&ValueSubkeyRangeSet::single(participant.subkey));
    }
    out
}
