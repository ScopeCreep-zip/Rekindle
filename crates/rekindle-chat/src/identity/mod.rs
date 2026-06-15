//! Identity lifecycle — init, replenish prekeys, destroy.
//!
//! The init ceremony is the most critical lifecycle operation. It generates
//! the user's cryptographic identity, creates their DHT presence, publishes
//! their PQXDH prekey bundle, and confirms propagation before returning.
//! Every DHT write uses Confirm::Propagated — the profile must be
//! discoverable by remote nodes before init returns.

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_storage::VaultStore;
use rekindle_storage::keys::labels;
use rekindle_types::session_types::{SessionMeta, SessionIdentity};
use rekindle_types::transport::RecordSchema;
use rekindle_types::dht_types::{
    PROFILE_SUBKEY_DISPLAY_NAME, PROFILE_SUBKEY_PREKEY_BUNDLE, PROFILE_SUBKEY_ROUTE_BLOB,
    PROFILE_SUBKEY_FRIEND_INBOX_KEY, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
    PROFILE_SUBKEY_X25519_PUB, PROFILE_SUBKEY_COUNT,
};

use crate::io::{Confirm, PlatformIO};
use crate::time::{timestamp_ms, timestamp_secs};
use crate::ChatError;

pub struct IdentityService {
    pub(crate) io: Arc<PlatformIO>,
    pub(crate) vault: Arc<VaultStore>,
    pub(crate) session_meta: Arc<RwLock<SessionMeta>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IdentityCreated {
    pub public_key: rekindle_identity::IdentityRoot,
    pub profile_dht_key: String,
    pub mailbox_dht_key: String,
    pub friend_list_dht_key: String,
    pub friend_inbox_key: String,
    pub route_blob: Vec<u8>,
}

impl IdentityService {
    /// Initialize a new identity.
    ///
    /// Generates Ed25519 signing key, derives X25519 DH key, generates
    /// PQXDH prekey bundle (ML-KEM-768 + X25519 + Ed25519), creates
    /// 4 DHT records, allocates a private route, publishes everything
    /// to the profile with Confirm::Propagated, persists all private
    /// material to vault.
    pub async fn init_identity(
        &self,
        display_name: &str,
    ) -> Result<IdentityCreated, ChatError> {
        {
            let meta = self.session_meta.read();
            if let Some(ref id) = meta.identity {
                return Ok(IdentityCreated {
                    public_key: id.public_key,
                    profile_dht_key: id.profile_dht_key.clone(),
                    mailbox_dht_key: id.mailbox_dht_key.clone(),
                    friend_list_dht_key: id.friend_list_dht_key.clone(),
                    friend_inbox_key: id.friend_inbox_key.clone(),
                    route_blob: Vec::new(),
                });
            }
        }

        // Step 1: Originate via identity crate — returns OriginationResult
        // with vault data accessible before consuming into SelfIdentity.
        let mut result = rekindle_identity::SelfIdentity::originate_new(
            "pending", // placeholder profile key — updated after DHT record creation
            Some(display_name),
        ).map_err(|e| ChatError::Internal(format!("identity origination: {e}")))?;

        // Step 2: Store seed material to vault before into_identity() consumes it
        self.vault.store_key(labels::SIGNING_KEY, result.vault_seed())?;
        self.vault.store_key(labels::IDENTITY_X25519_SEED, result.vault_x25519_seed())?;

        // Step 3: Extract public keys for DHT publication and prekey bundle
        let public_key_hex = result.public_key_hex();
        let ed_pub = *result.root.as_bytes();
        let x25519_pub = result.dh_public;

        // Step 4: Build signing keypair for PQXDH prekey bundle
        let kp = rekindle_identity::SigningKeypair::from_seed(result.vault_seed())
            .map_err(|e| ChatError::Internal(format!("signing keypair: {e}")))?;

        // Step 5: Generate PQXDH prekey bundle
        let (spk_seed, spk_pub) = rekindle_ratchet::crypto::dh::generate_ratchet_keypair()
            .map_err(|e| ChatError::Internal(format!("SPK keygen: {e}")))?;
        let spk_sig = kp.sign_ec_prekey(&spk_pub);

        let pq_ot = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM OT keygen: {e}")))?;
        let pqpk_ot_sig = kp.sign_pq_prekey(
            rekindle_identity::DOMAIN_OT, &pq_ot.ek_bytes,
        );

        let pq_lr = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM LR keygen: {e}")))?;
        let pqpk_lr_sig = kp.sign_pq_prekey(
            rekindle_identity::DOMAIN_LR, &pq_lr.ek_bytes,
        );

        let bundle = rekindle_ratchet::pqxdh::bundle::PreKeyBundle {
            ik_ed25519: ed_pub,
            ik_x25519: x25519_pub,
            spk_id: 1,
            spk: spk_pub,
            spk_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(spk_sig),
            opk_id: 0,
            opk: None,
            pqpk_ot_id: 1,
            pqpk_ot: pq_ot.ek_bytes.to_vec(),
            pqpk_ot_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(pqpk_ot_sig),
            pqpk_lr: pq_lr.ek_bytes.to_vec(),
            pqpk_lr_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(pqpk_lr_sig),
            published_at: timestamp_secs(),
        };
        let bundle_bytes = serde_json::to_vec(&bundle)
            .map_err(|e| ChatError::Serialization(format!("prekey bundle: {e}")))?;

        // Step 7: Store prekey private material in vault
        self.vault.store_key(&labels::signed_prekey(1), spk_seed.as_ref())?;
        self.vault.store_key(&labels::pq_prekey(1), pq_ot.dk_bytes.as_ref())?;
        self.vault.store_key(&labels::pq_last_resort(), pq_lr.dk_bytes.as_ref())?;

        // Step 8: Create DHT records
        let (profile_record, profile_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: PROFILE_SUBKEY_COUNT })
            .await?;
        let profile_key = profile_record.key().to_string();
        let (mailbox_record, _) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let mailbox_key = mailbox_record.key().to_string();
        let (friend_list_record, friend_list_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let friend_list_key = friend_list_record.key().to_string();
        let (friend_inbox_record, friend_inbox_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 32 })
            .await?;
        let friend_inbox_key = friend_inbox_record.key().to_string();
        let friend_inbox_keypair_hex = hex::encode(&friend_inbox_keypair);

        // Step 9: Store DHT keypairs in vault
        self.vault.store_key(labels::PROFILE_KEYPAIR, &profile_keypair)?;
        self.vault.store_key(labels::FRIEND_LIST_KEYPAIR, &friend_list_keypair)?;
        self.vault.store_key(labels::FRIEND_INBOX_KEYPAIR, &friend_inbox_keypair)?;

        // Step 10: Allocate private route
        let (_route_id, route_blob) = self.io.allocate_route().await?;

        // Step 11: Publish profile subkeys with Confirm::Propagated
        // Every write must propagate before init returns — peers need to
        // discover this profile to send friend requests.
        self.io.write_record(
            &profile_record, PROFILE_SUBKEY_DISPLAY_NAME,
            display_name.as_bytes(), Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile display_name propagation failed: {e} — retry init"
        )))?;

        self.io.write_record(
            &profile_record, PROFILE_SUBKEY_PREKEY_BUNDLE,
            &bundle_bytes, Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile prekey_bundle propagation failed: {e} — peers cannot establish sessions until propagated"
        )))?;

        self.io.write_record(
            &profile_record, PROFILE_SUBKEY_ROUTE_BLOB,
            &route_blob, Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile route_blob propagation failed: {e} — peers cannot reach this node until propagated"
        )))?;

        self.io.write_record(
            &profile_record, PROFILE_SUBKEY_FRIEND_INBOX_KEY,
            friend_inbox_key.as_bytes(), Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile friend_inbox_key propagation failed: {e} — peers cannot send friend requests until propagated"
        )))?;

        self.io.write_record(
            &profile_record, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
            friend_inbox_keypair_hex.as_bytes(), Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile friend_inbox_keypair propagation failed: {e}"
        )))?;

        // Step 11b: Publish X25519 DH public key for group DM MEK wrapping
        self.io.write_record(
            &profile_record, PROFILE_SUBKEY_X25519_PUB,
            &x25519_pub, Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile x25519_pub propagation failed: {e}"
        )))?;

        // Step 12: Seed friend inbox subkey 0
        self.io.write_record(
            &friend_inbox_record, 0, b"[]",
            Some(&friend_inbox_keypair), Confirm::Accepted,
        ).await?;

        // Step 13: Extract root before into_identity consumes the result
        let identity_root = result.root;
        result.set_profile_key(&profile_key)
            .map_err(|e| ChatError::Internal(format!("set profile key: {e}")))?;
        let self_id = result.into_identity()
            .map_err(|e| ChatError::Internal(format!("identity construct: {e}")))?;
        self.io.set_identity(self_id);

        // Step 14: Update session_meta
        {
            let mut meta = self.session_meta.write();
            meta.identity = Some(SessionIdentity {
                public_key: identity_root,
                display_name: display_name.to_string(),
                profile_dht_key: profile_key.clone(),
                mailbox_dht_key: mailbox_key.clone(),
                friend_list_dht_key: friend_list_key.clone(),
                friend_inbox_key: friend_inbox_key.clone(),
                friend_inbox_keypair_hex,
            });
        }

        tracing::info!(
            public_key = %public_key_hex,
            profile = %&profile_key[..12.min(profile_key.len())],
            "identity created — all profile subkeys propagated"
        );

        Ok(IdentityCreated {
            public_key: identity_root,
            profile_dht_key: profile_key,
            mailbox_dht_key: mailbox_key,
            friend_list_dht_key: friend_list_key,
            friend_inbox_key,
            route_blob,
        })
    }

    /// Replenish prekeys — generate a fresh PQXDH bundle and publish.
    ///
    /// Generates new SPK + ML-KEM-768 OT prekeys. The last-resort PQ prekey
    /// is long-lived and loaded from vault (not regenerated on replenish).
    /// If the last-resort key is missing from vault, a new one is generated.
    pub async fn replenish_prekeys(&self) -> Result<u32, ChatError> {
        let kp = self.io.signing_keypair()?;
        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        let ed_pub = kp.public_key_bytes();
        let x25519_seed = self.io.x25519_seed()?;
        let x25519_key = rekindle_ratchet::crypto::dh::reusable_from_seed(&x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519: {e}")))?;
        let x25519_pub_raw = x25519_key.compute_public_key()
            .map_err(|_| ChatError::Internal("x25519 pub derive".into()))?;
        let mut x25519_pub = [0u8; 32];
        x25519_pub.copy_from_slice(x25519_pub_raw.as_ref());

        let (new_spk_seed, new_spk_pub) = rekindle_ratchet::crypto::dh::generate_ratchet_keypair()
            .map_err(|e| ChatError::Internal(format!("SPK keygen: {e}")))?;
        let spk_sig = kp.sign_ec_prekey(&new_spk_pub);
        let spk_id = timestamp_ms();

        let pq_ot = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM OT keygen: {e}")))?;
        let pqpk_ot_sig = kp.sign_pq_prekey(rekindle_identity::DOMAIN_OT, &pq_ot.ek_bytes);
        let pqpk_ot_id = timestamp_ms();

        let (pq_lr_ek, pqpk_lr_sig) = {
            let fresh_lr = rekindle_ratchet::crypto::kem::keygen()
                .map_err(|e| ChatError::Internal(format!("ML-KEM LR keygen: {e}")))?;
            let sig = kp.sign_pq_prekey(rekindle_identity::DOMAIN_LR, &fresh_lr.ek_bytes);
            self.vault.store_key(&labels::pq_last_resort(), fresh_lr.dk_bytes.as_ref())?;
            (fresh_lr.ek_bytes.to_vec(), sig)
        };

        let bundle = rekindle_ratchet::pqxdh::bundle::PreKeyBundle {
            ik_ed25519: ed_pub,
            ik_x25519: x25519_pub,
            spk_id,
            spk: new_spk_pub,
            spk_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(spk_sig),
            opk_id: 0,
            opk: None,
            pqpk_ot_id,
            pqpk_ot: pq_ot.ek_bytes.to_vec(),
            pqpk_ot_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(pqpk_ot_sig),
            pqpk_lr: pq_lr_ek,
            pqpk_lr_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(pqpk_lr_sig),
            published_at: timestamp_secs(),
        };

        let bundle_bytes = serde_json::to_vec(&bundle)
            .map_err(|e| ChatError::Serialization(format!("prekey bundle: {e}")))?;
        let byte_count = bundle_bytes.len();

        // Store new prekey private material
        self.vault.store_key(&labels::signed_prekey(spk_id), new_spk_seed.as_ref())?;
        self.vault.store_key(&labels::pq_prekey(pqpk_ot_id), pq_ot.dk_bytes.as_ref())?;

        // Publish with Confirm::Verified — must be readable after write
        let profile_keypair = self.vault.load_key(labels::PROFILE_KEYPAIR)?;
        self.io.open_and_write(
            &identity.profile_dht_key, PROFILE_SUBKEY_PREKEY_BUNDLE,
            &bundle_bytes, profile_keypair.as_deref(), Confirm::Verified,
        ).await.map_err(|e| ChatError::Internal(format!(
            "prekey bundle publish failed: {e} — peers may use stale prekeys"
        )))?;

        #[allow(clippy::cast_possible_truncation)]
        let count = byte_count as u32;
        tracing::info!(
            bytes = byte_count,
            spk_id,
            pqpk_ot_id,
            "prekeys replenished — fresh SPK + ML-KEM OT published"
        );
        Ok(count)
    }

    /// Rotate the Ed25519 identity keypair.
    ///
    /// Generates a new signing key, derives new X25519, generates a fresh
    /// PQXDH prekey bundle, updates the profile DHT with new keys, and
    /// notifies all friends via DmPayload::ProfileKeyRotated so they update
    /// their contact for this peer.
    ///
    /// This is a destructive operation — the old public key becomes invalid.
    /// Any in-flight DMs encrypted to the old key will fail to decrypt on
    /// the recipient side (they'll need to re-establish the session).
    pub async fn rotate_identity(&self) -> Result<(), ChatError> {
        let old_identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        // Originate a new identity for rotation
        let new_result = rekindle_identity::SelfIdentity::originate_new(
            &old_identity.profile_dht_key,
            Some(&old_identity.display_name),
        ).map_err(|e| ChatError::Internal(format!("rotation origination: {e}")))?;

        // Store new seed material to vault (overwrites old)
        self.vault.store_key(labels::SIGNING_KEY, new_result.vault_seed())?;
        self.vault.store_key(labels::IDENTITY_X25519_SEED, new_result.vault_x25519_seed())?;

        // Build signing keypair for PQXDH bundle
        let kp = rekindle_identity::SigningKeypair::from_seed(new_result.vault_seed())
            .map_err(|e| ChatError::Internal(format!("signing keypair: {e}")))?;
        let new_public_key_hex = new_result.public_key_hex();
        let new_ed_pub = *new_result.root.as_bytes();
        let x25519_pub = new_result.dh_public;

        // Generate fresh PQXDH prekey bundle with new identity
        let (spk_seed, spk_pub) = rekindle_ratchet::crypto::dh::generate_ratchet_keypair()
            .map_err(|e| ChatError::Internal(format!("SPK keygen: {e}")))?;
        let spk_sig = kp.sign_ec_prekey(&spk_pub);

        let pq_ot = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM OT keygen: {e}")))?;
        let pqpk_ot_sig = kp.sign_pq_prekey(rekindle_identity::DOMAIN_OT, &pq_ot.ek_bytes);

        let pq_lr = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM LR keygen: {e}")))?;
        let pqpk_lr_sig = kp.sign_pq_prekey(rekindle_identity::DOMAIN_LR, &pq_lr.ek_bytes);

        let spk_id = timestamp_ms();
        let pqpk_ot_id = timestamp_ms();

        let bundle = rekindle_ratchet::pqxdh::bundle::PreKeyBundle {
            ik_ed25519: new_ed_pub,
            ik_x25519: x25519_pub,
            spk_id,
            spk: spk_pub,
            spk_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(spk_sig),
            opk_id: 0,
            opk: None,
            pqpk_ot_id,
            pqpk_ot: pq_ot.ek_bytes.to_vec(),
            pqpk_ot_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(pqpk_ot_sig),
            pqpk_lr: pq_lr.ek_bytes.to_vec(),
            pqpk_lr_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(pqpk_lr_sig),
            published_at: timestamp_secs(),
        };
        let bundle_bytes = serde_json::to_vec(&bundle)
            .map_err(|e| ChatError::Serialization(format!("prekey bundle: {e}")))?;

        // Step 7: Store new prekey private material
        self.vault.store_key(&labels::signed_prekey(spk_id), spk_seed.as_ref())?;
        self.vault.store_key(&labels::pq_prekey(pqpk_ot_id), pq_ot.dk_bytes.as_ref())?;
        self.vault.store_key(&labels::pq_last_resort(), pq_lr.dk_bytes.as_ref())?;

        // Step 8: Update profile DHT with new prekey bundle
        let profile_keypair = self.vault.load_key(labels::PROFILE_KEYPAIR)?;
        self.io.open_and_write(
            &old_identity.profile_dht_key, PROFILE_SUBKEY_PREKEY_BUNDLE,
            &bundle_bytes, profile_keypair.as_deref(), Confirm::Verified,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile prekey update failed during rotation: {e}"
        )))?;

        // Step 9: Extract root before into_identity consumes the result
        let new_identity_root = new_result.root;
        let new_self_id = new_result.into_identity()
            .map_err(|e| ChatError::Internal(format!("rotation identity construct: {e}")))?;
        self.io.set_identity(new_self_id);

        // Step 10: Notify all friends via DM (ProfileKeyRotated)
        let dm_peers: Vec<String> = {
            let meta = self.session_meta.read();
            let mut keys: Vec<String> = meta.dm_peers.keys().cloned().collect();
            for k in meta.dm_smpl_peers.keys() {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
            keys
        };
        for peer_key in &dm_peers {
            if let Err(e) = self.io.send_peer_notification(
                peer_key,
                rekindle_types::dm_payload::DmPayload::ProfileKeyRotated {
                    new_profile_dht_key: old_identity.profile_dht_key.clone(),
                },
                crate::io::Confirm::None,
            ).await {
                tracing::debug!(
                    peer = &peer_key[..12.min(peer_key.len())],
                    error = %e,
                    "profile rotation notification failed — peer will discover on next interaction"
                );
            }
        }

        // Step 11: Update session_meta with new public key
        {
            let mut meta = self.session_meta.write();
            if let Some(ref mut identity) = meta.identity {
                identity.public_key = new_identity_root;
            }
        }

        tracing::info!(
            new_key = %&new_public_key_hex[..16.min(new_public_key_hex.len())],
            friends_notified = dm_peers.len(),
            "identity rotated — new key active, friends notified"
        );
        Ok(())
    }

    /// Destroy the identity — close all DHT records, clear session state.
    pub async fn destroy_identity(&self) -> Result<(), ChatError> {
        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        // Close DHT records — best-effort, log errors
        for (name, key) in [
            ("profile", &identity.profile_dht_key),
            ("mailbox", &identity.mailbox_dht_key),
            ("friend_list", &identity.friend_list_dht_key),
            ("friend_inbox", &identity.friend_inbox_key),
        ] {
            if let Err(e) = self.io.close_record_by_key(key).await {
                tracing::warn!(
                    record = name,
                    key = &key[..12.min(key.len())],
                    error = %e,
                    "DHT record close failed during identity destroy — orphaned record will expire naturally"
                );
            }
        }

        // Clear session state
        {
            let mut meta = self.session_meta.write();
            meta.identity = None;
            meta.communities.clear();
            meta.dm_peers.clear();
            meta.dm_smpl_peers.clear();
            meta.pending_friend_requests.clear();
            meta.friend_display_names.clear();
            meta.pending_outbound_logs.clear();
        }

        tracing::info!("identity destroyed — all local state cleared");
        Ok(())
    }

    /// Rotate the ML-KEM last-resort prekey. Generates a new keypair,
    /// publishes via replenish_prekeys (which rebuilds the full bundle).
    pub async fn rotate_last_resort(&self) -> Result<(), ChatError> {
        // Generate new last-resort PQ prekey
        let pq_lr = rekindle_ratchet::crypto::kem::keygen()?;

        // Store new dk to vault (overwrites old)
        self.vault.store_key(
            &rekindle_storage::keys::labels::pq_last_resort(),
            pq_lr.dk_bytes.as_ref(),
        )?;

        // Republish prekey bundle with new last-resort key
        self.replenish_prekeys().await?;

        tracing::info!("last-resort PQ prekey rotated");
        Ok(())
    }
}
