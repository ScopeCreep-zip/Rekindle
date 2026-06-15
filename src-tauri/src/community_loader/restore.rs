//! Phase 23.C — `restore_community_pseudonyms_and_meks` lifted from
//! `commands/auth.rs`.
//!
//! Re-derive pseudonym keys and load MEKs from Stronghold into
//! `mek_cache`. Called during login after communities are loaded
//! from SQLite. For each community, derives the pseudonym
//! (deterministic from `identity_secret` + `community_id`) and loads
//! the MEK from Stronghold if stored.
//!
//! When the MEK is missing from Stronghold (vault loss), this does NOT
//! mint a replacement — minting forks the community key. The slot is left
//! absent and re-acquired from a peer by the architecture §7.3 login
//! catch-up in `login_runtime::spawn_dht_publish` (owner mints a
//! superseding key only as a last resort if no peer responds).

use crate::keystore::KeystoreHandle;
use crate::state::SharedState;

pub fn restore_community_pseudonyms_and_meks(
    state: &SharedState,
    keystore_handle: &KeystoreHandle,
    secret_key: &[u8; 32],
) {
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

    // Collect community IDs. Ownership for the recovery decision is
    // determined later (registry_owner_keypair) by the login catch-up, not here.
    let community_info: Vec<String> = {
        let communities = state.communities.read();
        communities.values().map(|c| c.id.clone()).collect()
    };

    let mut pseudonym_updates: Vec<(String, String)> = Vec::new();
    let mut mek_updates: Vec<(String, MediaEncryptionKey)> = Vec::new();
    let mut channel_mek_updates: Vec<(String, String, MediaEncryptionKey)> = Vec::new();

    for community_id in &community_info {
        // Derive pseudonym
        let signing_key = derive_community_pseudonym(secret_key, community_id);
        let pseudonym_hex = hex::encode(signing_key.verifying_key().as_bytes());
        pseudonym_updates.push((community_id.clone(), pseudonym_hex));

        // Try to load MEK from Stronghold
        let keystore = keystore_handle.lock();
        if let Some(ref ks) = *keystore {
            if let Some(mek) = crate::keystore::load_mek(ks, community_id) {
                mek_updates.push((community_id.clone(), mek));
            } else {
                // MEK missing from Stronghold (vault loss). Do NOT mint a fresh
                // key here — that forks the community (peers hold the canonical
                // key; a local mint diverges and the convergence rule can't
                // reconcile two untagged same-generation keys). Leave the slot
                // ABSENT; the architecture §7.3 login catch-up in
                // `login_runtime::spawn_dht_publish` re-acquires the canonical
                // key from an online peer, and (owner-only) mints a SUPERSEDING
                // key as a last resort if no peer can serve it. `c.mek_generation`
                // is left untouched so it remains the acquisition target.
                tracing::warn!(
                    community = %community_id,
                    "MEK missing from Stronghold — will be re-acquired from a peer \
                     on login (owner mints a superseding key only if none responds)"
                );
            }
        }
    }

    {
        let communities = state.communities.read();
        let keystore = keystore_handle.lock();
        if let Some(ref ks) = *keystore {
            for community in communities.values() {
                for channel in &community.channels {
                    let all = crate::keystore::load_all_meks(ks, &community.id, Some(&channel.id));
                    if let Some(mek) = all.into_iter().max_by_key(
                        rekindle_crypto::group::media_key::MediaEncryptionKey::generation,
                    ) {
                        channel_mek_updates.push((community.id.clone(), channel.id.clone(), mek));
                    }
                }
            }
        }
    }

    // Load slot/registry key material from Stronghold
    let mut slot_keypair_updates: Vec<(String, String)> = Vec::new();
    let mut slot_seed_updates: Vec<(String, String)> = Vec::new();
    let mut registry_keypair_updates: Vec<(String, String)> = Vec::new();
    {
        let keystore = keystore_handle.lock();
        if let Some(ref ks) = *keystore {
            for community_id in &community_info {
                if let Some(kp) = crate::keystore::load_slot_keypair(ks, community_id) {
                    slot_keypair_updates.push((community_id.clone(), kp));
                }
                if let Some(seed) = crate::keystore::load_slot_seed(ks, community_id) {
                    slot_seed_updates.push((community_id.clone(), seed));
                }
                if let Some(rkp) = crate::keystore::load_registry_keypair(ks, community_id) {
                    registry_keypair_updates.push((community_id.clone(), rkp));
                }
            }
        }
    }

    // Update communities with derived pseudonyms + keypairs
    {
        let mut communities = state.communities.write();
        for (community_id, pseudonym_hex) in pseudonym_updates {
            if let Some(c) = communities.get_mut(&community_id) {
                if c.my_pseudonym_key.is_none() {
                    c.my_pseudonym_key = Some(pseudonym_hex);
                }
            }
        }

        for (community_id, kp) in slot_keypair_updates {
            if let Some(c) = communities.get_mut(&community_id) {
                c.slot_keypair = Some(kp);
            }
        }
        for (community_id, seed) in slot_seed_updates {
            if let Some(c) = communities.get_mut(&community_id) {
                c.slot_seed = Some(seed);
            }
        }
        for (community_id, rkp) in registry_keypair_updates {
            if let Some(c) = communities.get_mut(&community_id) {
                c.registry_owner_keypair = Some(rkp);
            }
        }
    }

    // Load MEKs into cache via the centralized resolvers (NOT a raw insert):
    // restored keys carry provenance (Stronghold round-trips it via
    // to/from_wire_bytes), so a restore that races a live rotation converges
    // deterministically instead of clobbering. Call per-item — the helpers
    // take their own lock, so we must not hold the cache lock here.
    for (community_id, mek) in mek_updates {
        tracing::debug!(
            community = %community_id,
            generation = mek.generation(),
            "restored MEK from Stronghold"
        );
        crate::state_helpers::install_community_mek(state, &community_id, mek);
    }

    for (community_id, channel_id, mek) in channel_mek_updates {
        tracing::debug!(
            community = %community_id,
            channel = %channel_id,
            generation = mek.generation(),
            "restored channel MEK from Stronghold"
        );
        crate::state_helpers::install_channel_mek(state, &community_id, &channel_id, mek);
    }
}
