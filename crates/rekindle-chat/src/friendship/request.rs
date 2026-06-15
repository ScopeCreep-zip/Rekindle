//! Send a friend request — create DhtLog, sign entry, write to inbox,
//! verify propagation, send direct notification.

use rekindle_identity::signing::{DOMAIN_OT, DOMAIN_LR};
use rekindle_storage::keys::labels;
use rekindle_types::dht_types::{
    FriendRequestEntry, FriendRequestStatus, PROFILE_SUBKEY_FRIEND_INBOX_KEY,
    PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
};
use rekindle_types::transport::RecordSchema;

use crate::io::Confirm;
use crate::time::{timestamp_ms, timestamp_secs};
use crate::ChatError;
use super::FriendshipService;

/// Result of sending a friend request.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FriendRequestSent {
    pub dm_log_key: String,
    pub target: String,
}

impl FriendshipService {
    /// Send a friend request to a target identified by their profile DHT key.
    ///
    /// 1. Read target's friend inbox key from their profile DHT
    /// 2. Generate PQXDH prekey bundle (Ed25519 + X25519 + ML-KEM-768)
    /// 3. Create outbound DhtLog for DMs
    /// 4. Build + sign FriendRequestEntry
    /// 5. Write to target's inbox with Confirm::Verified (read-back verification)
    /// 6. Persist prekey material + DhtLog keypair to vault
    /// 7. Store pending_outbound_logs bridge in vault
    /// 8. Best-effort direct notification via PlatformIO
    pub async fn send_friend_request(
        &self,
        target_profile_key: &str,
        message: &str,
    ) -> Result<FriendRequestSent, ChatError> {
        let identity = self.require_identity()?;

        // Idempotent: if already sent a pending request to this target, return existing.
        {
            let meta = self.session_meta.read();
            if let Some(dm_log_key) = meta.pending_outbound_logs.get(target_profile_key) {
                return Ok(FriendRequestSent {
                    dm_log_key: dm_log_key.clone(),
                    target: target_profile_key.to_string(),
                });
            }
        }

        // Step 1: Read target's friend inbox key + keypair from their profile
        let target_profile = self.io.open_record(target_profile_key, None).await?;

        let inbox_key_data = self.io
            .read_record(&target_profile, PROFILE_SUBKEY_FRIEND_INBOX_KEY, true)
            .await?
            .ok_or(ChatError::InboxNotAvailable)?;
        let inbox_keypair_data = self.io
            .read_record(&target_profile, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR, true)
            .await?
            .ok_or(ChatError::InboxNotAvailable)?;

        let inbox_key = String::from_utf8_lossy(&inbox_key_data).to_string();
        let inbox_keypair_hex = String::from_utf8_lossy(&inbox_keypair_data).to_string();
        if inbox_key.is_empty() || inbox_keypair_hex.is_empty() {
            return Err(ChatError::InboxNotAvailable);
        }

        let inbox_kp_bytes = hex::decode(&inbox_keypair_hex)
            .map_err(|e| ChatError::Internal(format!("inbox keypair hex: {e}")))?;

        // Discover and cache the target's route blob for direct messaging.
        // This populates the peer registry so send_peer_notification works.
        self.io.discover_peer_route(target_profile_key, target_profile_key, None).await?;

        // Step 2: Generate PQXDH PreKeyBundle using identity crate
        let kp = self.io.signing_keypair()?;
        let ed_pub = kp.public_key_bytes();

        // X25519 identity DH key — derived deterministically from identity seed
        let x25519_seed = self.io.x25519_identity_seed()?;
        let x25519_key = rekindle_ratchet::crypto::dh::reusable_from_seed(&x25519_seed)?;
        let x25519_pub_raw = x25519_key.compute_public_key()
            .map_err(|_| ChatError::Internal("x25519 pubkey derive failed".into()))?;
        let mut x25519_pub = [0u8; 32];
        x25519_pub.copy_from_slice(x25519_pub_raw.as_ref());

        // Signed prekey (X25519)
        let (spk_seed, spk_pub) = rekindle_ratchet::crypto::dh::generate_ratchet_keypair()?;
        let spk_sig = kp.sign_ec_prekey(&spk_pub);
        let spk_id = 1u64;

        // ML-KEM-768 one-time PQ prekey
        let pq_material = rekindle_ratchet::crypto::kem::keygen()?;
        let pqpk_ot_sig = kp.sign_pq_prekey(DOMAIN_OT, &pq_material.ek_bytes);
        let pqpk_ot_id = 1u64;

        // ML-KEM-768 last-resort PQ prekey
        let pq_lr_material = rekindle_ratchet::crypto::kem::keygen()?;
        let pqpk_lr_sig = kp.sign_pq_prekey(DOMAIN_LR, &pq_lr_material.ek_bytes);

        let bundle = rekindle_ratchet::pqxdh::bundle::PreKeyBundle {
            ik_ed25519: ed_pub,
            ik_x25519: x25519_pub,
            spk_id,
            spk: spk_pub,
            spk_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(spk_sig),
            opk_id: 0,
            opk: None,
            pqpk_ot_id,
            pqpk_ot: pq_material.ek_bytes.to_vec(),
            pqpk_ot_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(pqpk_ot_sig),
            pqpk_lr: pq_lr_material.ek_bytes.to_vec(),
            pqpk_lr_signature: rekindle_ratchet::pqxdh::bundle::Signature::from_bytes(pqpk_lr_sig),
            published_at: timestamp_secs(),
        };

        let prekey_bytes = serde_json::to_vec(&bundle)
            .map_err(|e| ChatError::Serialization(format!("prekey bundle: {e}")))?;

        // Persist prekey private material to vault for PQXDH completion
        // across restarts. Label keyed by profile DHT key (VLD0: stripped).
        self.vault.store_key(
            &labels::target_signed_prekey(target_profile_key),
            spk_seed.as_ref(),
        )?;
        self.vault.store_key(
            &labels::target_pq_prekey(target_profile_key),
            pq_material.dk_bytes.as_ref(),
        )?;
        self.vault.store_key(
            &labels::target_pq_last_resort(target_profile_key),
            pq_lr_material.dk_bytes.as_ref(),
        )?;

        // Step 3: Create outbound DhtLog (we write our DMs here, peer reads)
        let (dm_log_record, dm_log_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let dm_log_key = dm_log_record.key().to_string();
        let dm_log_keypair_hex = hex::encode(&dm_log_keypair);

        // Step 4: Build + sign FriendRequestEntry
        let mut entry = FriendRequestEntry {
            sender_public_key: identity.public_key,
            display_name: identity.display_name.clone(),
            message: message.to_string(),
            profile_dht_key: identity.profile_dht_key.clone(),
            mailbox_dht_key: identity.mailbox_dht_key.clone(),
            sender_friend_inbox_key: identity.friend_inbox_key.clone(),
            sender_friend_inbox_keypair_hex: identity.friend_inbox_keypair_hex.clone(),
            prekey_bundle: prekey_bytes,
            sent_at: timestamp_ms(),
            dm_log_key: dm_log_key.clone(),
            dm_log_keypair_hex: dm_log_keypair_hex.clone(),
            x25519_pub_hex: hex::encode(x25519_pub),
            signature_hex: String::new(),
            status: FriendRequestStatus::Pending,
        };

        let content = entry.signature_content();
        let sig = kp.sign_ec_prekey(&content);
        entry.signature_hex = hex::encode(sig);

        // Step 5: Write to target's inbox with read-back verification
        let subkey = blake3_hash_mod(&identity.public_key.to_hex(), target_profile_key, 32);

        let inbox_record = self.io.open_record(&inbox_key, Some(&inbox_kp_bytes)).await?;

        let existing = self.io
            .read_record(&inbox_record, subkey, true)
            .await?
            .unwrap_or_default();

        let mut entries: Vec<FriendRequestEntry> = if existing.is_empty() || existing == b"[]" {
            Vec::new()
        } else {
            FriendRequestEntry::parse_inbox_data(&existing).unwrap_or_default()
        };
        entries.retain(|e| e.sender_public_key != entry.sender_public_key);
        entries.push(entry.clone());

        let bytes = serde_json::to_vec(&entries)
            .map_err(|e| ChatError::Serialization(format!("inbox entry: {e}")))?;

        let receipt = self.io
            .write_record(&inbox_record, subkey, &bytes, Some(&inbox_kp_bytes), Confirm::Propagated)
            .await?;

        let target_short = &target_profile_key[..12.min(target_profile_key.len())];
        if receipt.remote_holders > 0 {
            tracing::info!(
                target = target_short,
                remote_holders = receipt.remote_holders,
                elapsed_ms = receipt.elapsed.as_millis(),
                "friend request propagated — confirmed by remote nodes"
            );
        } else {
            tracing::warn!(
                target = target_short,
                elapsed_ms = receipt.elapsed.as_millis(),
                "friend request written but propagation unconfirmed — \
                 target may experience delay discovering this request"
            );
        }

        // Step 6: Persist DhtLog keypair to vault
        let log_short = &dm_log_key[..12.min(dm_log_key.len())];
        self.vault
            .store_key(&labels::dm_log_keypair(log_short), &dm_log_keypair)?;

        // Step 7: Store pending_outbound_logs bridge in vault
        self.vault
            .store_pending_outbound(target_profile_key, &dm_log_key)?;

        // Update session meta
        {
            let mut meta = self.session_meta.write();
            meta.pending_outbound_logs
                .insert(target_profile_key.to_string(), dm_log_key.clone());
        }

        // Step 8: Best-effort direct notification via PlatformIO
        if let Err(e) = self.io.send_peer_notification(
            target_profile_key,
            rekindle_types::dm_payload::DmPayload::FriendRequestAck,
            Confirm::None,
        ).await {
            tracing::debug!(
                target = target_short,
                error = %e,
                "direct friend request notification failed — \
                 target will discover via inbox scan (reliable, slower)"
            );
        }

        tracing::info!(
            target = target_short,
            dm_log = &dm_log_key[..12.min(dm_log_key.len())],
            "friend request sent"
        );

        Ok(FriendRequestSent {
            dm_log_key,
            target: target_profile_key.to_string(),
        })
    }
}

/// Deterministic subkey index from sender + recipient keys.
pub fn blake3_hash_mod(sender: &str, recipient: &str, n: u32) -> u32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(sender.as_bytes());
    hasher.update(b"|");
    hasher.update(recipient.as_bytes());
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % n
}
