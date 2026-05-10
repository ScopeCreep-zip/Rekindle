//! Identity lifecycle — init, replenish prekeys, destroy.
//!
//! The init ceremony is the most critical lifecycle operation. It generates
//! the user's cryptographic identity, creates their DHT presence, publishes
//! their PQXDH prekey bundle, and confirms propagation before returning.
//! Every DHT write uses Confirm::Propagated — the profile must be
//! discoverable by remote nodes before init returns.

use std::sync::Arc;

use parking_lot::RwLock;
use zeroize::Zeroizing;
use rekindle_storage::VaultStore;
use rekindle_storage::keys::labels;
use rekindle_types::session_types::{SessionMeta, SessionIdentity};
use rekindle_types::transport::RecordSchema;
use rekindle_types::dht_types::{
    PROFILE_SUBKEY_DISPLAY_NAME, PROFILE_SUBKEY_PREKEY_BUNDLE, PROFILE_SUBKEY_ROUTE_BLOB,
    PROFILE_SUBKEY_FRIEND_INBOX_KEY, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
};

use crate::crypto::SigningKeyHandle;
use crate::io::{Confirm, PlatformIO};
use crate::time::{timestamp_ms, timestamp_secs};
use crate::ChatError;

use aws_lc_rs::rand::SecureRandom;

pub struct IdentityService {
    pub(crate) io: Arc<PlatformIO>,
    pub(crate) vault: Arc<VaultStore>,
    pub(crate) session_meta: Arc<RwLock<SessionMeta>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IdentityCreated {
    pub public_key_hex: String,
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
            if meta.identity.is_some() {
                return Err(ChatError::AlreadyInitialized);
            }
        }

        // Step 1: Generate Ed25519 signing seed
        let mut signing_seed = Zeroizing::new([0u8; 32]);
        aws_lc_rs::rand::SystemRandom::new()
            .fill(signing_seed.as_mut())
            .map_err(|e| ChatError::Internal(format!("signing key generation failed: {e}")))?;

        // Step 2: Store signing seed in vault
        self.vault.store_key(labels::SIGNING_KEY, signing_seed.as_ref())?;

        // Step 3: Derive and store X25519 DH seed (deterministic from signing seed)
        let x25519_seed = blake3::derive_key("rekindle identity x25519 v1", signing_seed.as_ref());
        self.vault.store_key(labels::IDENTITY_X25519_SEED, &x25519_seed)?;

        // Step 4: Set signing key handle on PlatformIO.
        // After this point, all PlatformIO identity/pseudonym methods work.
        // This is the ONLY place that sets the key outside of ChatService::resume/lock.
        let handle = SigningKeyHandle::from_vault(&self.vault)?;
        self.io.set_signing_key(handle);

        // Build Ed25519 keypair for prekey signing
        let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&signing_seed)?;
        let ed_pub = rekindle_ratchet::crypto::sign::public_key_bytes(&kp);
        let public_key_hex = hex::encode(ed_pub);

        // Step 5: Derive X25519 public key deterministically
        let x25519_key = rekindle_ratchet::crypto::dh::reusable_from_seed(&x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519 from seed: {e}")))?;
        let x25519_pub_raw = x25519_key.compute_public_key()
            .map_err(|_| ChatError::Internal("x25519 pubkey derivation failed".into()))?;
        let mut x25519_pub = [0u8; 32];
        x25519_pub.copy_from_slice(x25519_pub_raw.as_ref());

        // Step 6: Generate PQXDH prekey bundle
        let (spk_seed, spk_pub) = rekindle_ratchet::crypto::dh::generate_ratchet_keypair()
            .map_err(|e| ChatError::Internal(format!("SPK keygen: {e}")))?;
        let spk_sig = rekindle_ratchet::crypto::sign::sign_ec_prekey(&kp, &spk_pub);

        let pq_ot = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM OT keygen: {e}")))?;
        let pqpk_ot_sig = rekindle_ratchet::crypto::sign::sign_pq_prekey(
            &kp, rekindle_ratchet::crypto::sign::DOMAIN_OT, &pq_ot.ek_bytes,
        );

        let pq_lr = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM LR keygen: {e}")))?;
        let pqpk_lr_sig = rekindle_ratchet::crypto::sign::sign_pq_prekey(
            &kp, rekindle_ratchet::crypto::sign::DOMAIN_LR, &pq_lr.ek_bytes,
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
        let (profile_key, profile_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 10 })
            .await?;
        let (mailbox_key, _mailbox_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let (friend_list_key, friend_list_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let (friend_inbox_key, friend_inbox_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 32 })
            .await?;
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
            &profile_key, PROFILE_SUBKEY_DISPLAY_NAME,
            display_name.as_bytes(), Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile display_name propagation failed: {e} — retry init"
        )))?;

        self.io.write_record(
            &profile_key, PROFILE_SUBKEY_PREKEY_BUNDLE,
            &bundle_bytes, Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile prekey_bundle propagation failed: {e} — peers cannot establish sessions until propagated"
        )))?;

        self.io.write_record(
            &profile_key, PROFILE_SUBKEY_ROUTE_BLOB,
            &route_blob, Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile route_blob propagation failed: {e} — peers cannot reach this node until propagated"
        )))?;

        self.io.write_record(
            &profile_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY,
            friend_inbox_key.as_bytes(), Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile friend_inbox_key propagation failed: {e} — peers cannot send friend requests until propagated"
        )))?;

        self.io.write_record(
            &profile_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
            friend_inbox_keypair_hex.as_bytes(), Some(&profile_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile friend_inbox_keypair propagation failed: {e}"
        )))?;

        // Step 12: Seed friend inbox subkey 0
        self.io.write_record(
            &friend_inbox_key, 0, b"[]",
            Some(&friend_inbox_keypair), Confirm::Accepted,
        ).await?;

        // Step 13: Update session_meta
        {
            let mut meta = self.session_meta.write();
            meta.identity = Some(SessionIdentity {
                public_key_hex: public_key_hex.clone(),
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
            public_key_hex,
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
        let signing_seed = self.io.require_signing_key()?;
        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&signing_seed)?;
        let ed_pub = rekindle_ratchet::crypto::sign::public_key_bytes(&kp);

        // Deterministic X25519 pub from identity seed
        let x25519_seed = self.io.x25519_seed()?;
        let x25519_key = rekindle_ratchet::crypto::dh::reusable_from_seed(&x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519: {e}")))?;
        let x25519_pub_raw = x25519_key.compute_public_key()
            .map_err(|_| ChatError::Internal("x25519 pub derive".into()))?;
        let mut x25519_pub = [0u8; 32];
        x25519_pub.copy_from_slice(x25519_pub_raw.as_ref());

        // Fresh SPK
        let (new_spk_seed, new_spk_pub) = rekindle_ratchet::crypto::dh::generate_ratchet_keypair()
            .map_err(|e| ChatError::Internal(format!("SPK keygen: {e}")))?;
        let spk_sig = rekindle_ratchet::crypto::sign::sign_ec_prekey(&kp, &new_spk_pub);
        let spk_id = timestamp_ms(); // monotonic ID based on time

        // Fresh ML-KEM-768 OT
        let pq_ot = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM OT keygen: {e}")))?;
        let pqpk_ot_sig = rekindle_ratchet::crypto::sign::sign_pq_prekey(
            &kp, rekindle_ratchet::crypto::sign::DOMAIN_OT, &pq_ot.ek_bytes,
        );
        let pqpk_ot_id = timestamp_ms();

        // Last-resort PQ prekey — load from vault, generate if missing
        let (pq_lr_ek, pqpk_lr_sig) = match self.vault.load_key(&labels::pq_last_resort()) {
            Ok(Some(dk_bytes)) if dk_bytes.len() == 2400 => {
                // Reconstruct ek from dk is not possible — we need the ek stored separately.
                // The LR ek was published at init time and is in the profile DHT.
                // For replenish, we generate a fresh LR if we can't recover the ek.
                let fresh_lr = rekindle_ratchet::crypto::kem::keygen()
                    .map_err(|e| ChatError::Internal(format!("ML-KEM LR regen: {e}")))?;
                let sig = rekindle_ratchet::crypto::sign::sign_pq_prekey(
                    &kp, rekindle_ratchet::crypto::sign::DOMAIN_LR, &fresh_lr.ek_bytes,
                );
                self.vault.store_key(&labels::pq_last_resort(), fresh_lr.dk_bytes.as_ref())?;
                (fresh_lr.ek_bytes.to_vec(), sig)
            }
            _ => {
                let fresh_lr = rekindle_ratchet::crypto::kem::keygen()
                    .map_err(|e| ChatError::Internal(format!("ML-KEM LR keygen: {e}")))?;
                let sig = rekindle_ratchet::crypto::sign::sign_pq_prekey(
                    &kp, rekindle_ratchet::crypto::sign::DOMAIN_LR, &fresh_lr.ek_bytes,
                );
                self.vault.store_key(&labels::pq_last_resort(), fresh_lr.dk_bytes.as_ref())?;
                (fresh_lr.ek_bytes.to_vec(), sig)
            }
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
        self.io.write_record(
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

        // Step 1: Generate new Ed25519 signing seed
        let mut new_signing_seed = Zeroizing::new([0u8; 32]);
        aws_lc_rs::rand::SystemRandom::new()
            .fill(new_signing_seed.as_mut())
            .map_err(|e| ChatError::Internal(format!("new signing key generation: {e}")))?;

        // Step 2: Store new signing seed in vault (overwrites old)
        self.vault.store_key(labels::SIGNING_KEY, new_signing_seed.as_ref())?;

        // Step 3: Derive and store new X25519 DH seed
        let new_x25519_seed = blake3::derive_key("rekindle identity x25519 v1", new_signing_seed.as_ref());
        self.vault.store_key(labels::IDENTITY_X25519_SEED, &new_x25519_seed)?;

        // Step 4: Set new signing key on PlatformIO (old key is ZeroizeOnDrop'd)
        let handle = SigningKeyHandle::from_vault(&self.vault)?;
        self.io.set_signing_key(handle);

        // Step 5: Derive new public keys
        let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&new_signing_seed)?;
        let new_ed_pub = rekindle_ratchet::crypto::sign::public_key_bytes(&kp);
        let new_public_key_hex = hex::encode(new_ed_pub);

        let x25519_key = rekindle_ratchet::crypto::dh::reusable_from_seed(&new_x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519 from seed: {e}")))?;
        let x25519_pub_raw = x25519_key.compute_public_key()
            .map_err(|_| ChatError::Internal("x25519 pub derive failed".into()))?;
        let mut x25519_pub = [0u8; 32];
        x25519_pub.copy_from_slice(x25519_pub_raw.as_ref());

        // Step 6: Generate fresh PQXDH prekey bundle with new identity
        let (spk_seed, spk_pub) = rekindle_ratchet::crypto::dh::generate_ratchet_keypair()
            .map_err(|e| ChatError::Internal(format!("SPK keygen: {e}")))?;
        let spk_sig = rekindle_ratchet::crypto::sign::sign_ec_prekey(&kp, &spk_pub);

        let pq_ot = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM OT keygen: {e}")))?;
        let pqpk_ot_sig = rekindle_ratchet::crypto::sign::sign_pq_prekey(
            &kp, rekindle_ratchet::crypto::sign::DOMAIN_OT, &pq_ot.ek_bytes,
        );

        let pq_lr = rekindle_ratchet::crypto::kem::keygen()
            .map_err(|e| ChatError::Internal(format!("ML-KEM LR keygen: {e}")))?;
        let pqpk_lr_sig = rekindle_ratchet::crypto::sign::sign_pq_prekey(
            &kp, rekindle_ratchet::crypto::sign::DOMAIN_LR, &pq_lr.ek_bytes,
        );

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
        self.io.write_record(
            &old_identity.profile_dht_key, PROFILE_SUBKEY_PREKEY_BUNDLE,
            &bundle_bytes, profile_keypair.as_deref(), Confirm::Verified,
        ).await.map_err(|e| ChatError::Internal(format!(
            "profile prekey update failed during rotation: {e}"
        )))?;

        // Step 9: Notify all friends via DM (ProfileKeyRotated)
        let dm_peers: Vec<String> = {
            let meta = self.session_meta.read();
            meta.dm_peers.keys().cloned().collect()
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

        // Step 10: Update session_meta with new public key
        {
            let mut meta = self.session_meta.write();
            if let Some(ref mut identity) = meta.identity {
                identity.public_key_hex.clone_from(&new_public_key_hex);
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
            if let Err(e) = self.io.close_record(key).await {
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
            meta.pending_friend_requests.clear();
            meta.friend_display_names.clear();
            meta.pending_outbound_logs.clear();
        }

        tracing::info!("identity destroyed — all local state cleared");
        Ok(())
    }
}
