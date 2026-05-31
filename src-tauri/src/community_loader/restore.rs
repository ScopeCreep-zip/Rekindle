//! Phase 23.C — `restore_community_pseudonyms_and_meks` lifted from
//! `commands/auth.rs`.
//!
//! Re-derive pseudonym keys and load MEKs from Stronghold into
//! `mek_cache`. Called during login after communities are loaded
//! from SQLite. For each community, derives the pseudonym
//! (deterministic from `identity_secret` + `community_id`) and loads
//! the MEK from Stronghold if stored.
//!
//! For **hosted** (owned) communities where the MEK is missing from
//! Stronghold (e.g. communities created before MEK persistence was
//! added), a fresh MEK is regenerated and immediately persisted so
//! subsequent restarts succeed.

use crate::keystore::KeystoreHandle;
use crate::state::SharedState;

pub fn restore_community_pseudonyms_and_meks(
    state: &SharedState,
    keystore_handle: &KeystoreHandle,
    secret_key: &[u8; 32],
) {
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

    // Collect community IDs and whether we own them (have dht_owner_keypair)
    let community_info: Vec<(String, bool)> = {
        let communities = state.communities.read();
        communities
            .values()
            .map(|c| (c.id.clone(), c.dht_owner_keypair.is_some()))
            .collect()
    };

    let mut pseudonym_updates: Vec<(String, String)> = Vec::new();
    let mut mek_updates: Vec<(String, MediaEncryptionKey)> = Vec::new();
    let mut channel_mek_updates: Vec<(String, String, MediaEncryptionKey)> = Vec::new();
    let mut regenerated_community_ids: Vec<String> = Vec::new();

    for (community_id, is_owner) in &community_info {
        // Derive pseudonym
        let signing_key = derive_community_pseudonym(secret_key, community_id);
        let pseudonym_hex = hex::encode(signing_key.verifying_key().as_bytes());
        pseudonym_updates.push((community_id.clone(), pseudonym_hex));

        // Try to load MEK from Stronghold
        let keystore = keystore_handle.lock();
        if let Some(ref ks) = *keystore {
            if let Some(mek) = crate::keystore::load_mek(ks, community_id) {
                mek_updates.push((community_id.clone(), mek));
            } else if *is_owner {
                tracing::warn!(
                    community = %community_id,
                    "MEK missing from Stronghold for owned community — regenerating"
                );
                let mek = MediaEncryptionKey::generate(1);
                crate::keystore::persist_mek(ks, community_id, &mek);
                mek_updates.push((community_id.clone(), mek));
                regenerated_community_ids.push(community_id.clone());
            } else {
                tracing::warn!(
                    community = %community_id,
                    "MEK missing from Stronghold for joined community — \
                     will be delivered when connecting to an online member"
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
                    let all =
                        crate::keystore::load_all_meks(ks, &community.id, Some(&channel.id));
                    if let Some(mek) = all.into_iter().max_by_key(
                        rekindle_crypto::group::media_key::MediaEncryptionKey::generation,
                    ) {
                        channel_mek_updates.push((
                            community.id.clone(),
                            channel.id.clone(),
                            mek,
                        ));
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
            for (community_id, _) in &community_info {
                if let Some(kp) = crate::keystore::load_slot_keypair(ks, community_id) {
                    slot_keypair_updates.push((community_id.clone(), kp));
                }
                if let Some(seed) = crate::keystore::load_slot_seed(ks, community_id) {
                    slot_seed_updates.push((community_id.clone(), seed));
                }
                if let Some(rkp) =
                    crate::keystore::load_registry_keypair(ks, community_id)
                {
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

        for community_id in &regenerated_community_ids {
            if let Some(c) = communities.get_mut(community_id) {
                c.mek_generation = 1;
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

    // Load MEKs into cache
    {
        let mut mek_cache = state.mek_cache.lock();
        for (community_id, mek) in mek_updates {
            tracing::debug!(
                community = %community_id,
                generation = mek.generation(),
                "restored MEK from Stronghold"
            );
            mek_cache.insert(community_id, mek);
        }
    }

    {
        let mut channel_mek_cache = state.channel_mek_cache.lock();
        for (community_id, channel_id, mek) in channel_mek_updates {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                generation = mek.generation(),
                "restored channel MEK from Stronghold"
            );
            channel_mek_cache.insert((community_id, channel_id), mek);
        }
    }
}
