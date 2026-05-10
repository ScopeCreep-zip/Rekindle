//! Reject a pending friend request.

use rekindle_ratchet::crypto::sign;
use rekindle_types::dht_types::{
    FriendRequestEntry, FriendRequestStatus,
    PROFILE_SUBKEY_FRIEND_INBOX_KEY, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
};

use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::FriendshipService;
use super::request::blake3_hash_mod;

impl FriendshipService {
    /// Reject a pending friend request.
    ///
    /// Writes a Rejected entry to the requester's inbox so they see the rejection.
    /// Removes the request from our pending list.
    pub async fn reject_friend_request(
        &self,
        peer_pubkey: &str,
    ) -> Result<(), ChatError> {
        let signing_seed = self.io.require_signing_key()?;
        let identity = self.require_identity()?;

        let request = {
            let meta = self.session_meta.read();
            meta.pending_request_by_key(peer_pubkey)
                .cloned()
                .ok_or_else(|| ChatError::RequestNotFound {
                    peer_key: peer_pubkey.to_string(),
                })?
        };

        // Read requester's inbox key from their profile
        self.io.open_record(&request.profile_dht_key, None).await?;

        let req_inbox_key = self.io
            .read_record(&request.profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY, true)
            .await?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        let req_inbox_kp_hex = self.io
            .read_record(&request.profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR, true)
            .await?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();

        if !req_inbox_key.is_empty() && !req_inbox_kp_hex.is_empty() {
            if let Ok(kp_bytes) = hex::decode(&req_inbox_kp_hex) {
                self.io.open_record(&req_inbox_key, Some(&kp_bytes)).await?;

                let kp = sign::keypair_from_seed(&signing_seed)?;
                let mut response = FriendRequestEntry {
                    sender_public_key: identity.public_key_hex.clone(),
                    display_name: identity.display_name.clone(),
                    message: String::new(),
                    profile_dht_key: identity.profile_dht_key.clone(),
                    mailbox_dht_key: identity.mailbox_dht_key.clone(),
                    sender_friend_inbox_key: String::new(),
                    sender_friend_inbox_keypair_hex: String::new(),
                    prekey_bundle: Vec::new(),
                    sent_at: timestamp_ms(),
                    dm_log_key: String::new(),
                    dm_log_keypair_hex: String::new(),
                    x25519_pub_hex: String::new(),
                    signature_hex: String::new(),
                    status: FriendRequestStatus::Rejected {
                        rejected_at: timestamp_ms(),
                    },
                };

                let content = response.signature_content();
                let sig = sign::sign_ec_prekey(&kp, &content);
                response.signature_hex = hex::encode(sig);

                let subkey = blake3_hash_mod(
                    &identity.public_key_hex,
                    &request.profile_dht_key,
                    32,
                );

                let existing = self.io
                    .read_record(&req_inbox_key, subkey, true)
                    .await?
                    .unwrap_or_default();

                let mut entries: Vec<FriendRequestEntry> =
                    if existing.is_empty() || existing == b"[]" {
                        Vec::new()
                    } else {
                        FriendRequestEntry::parse_inbox_data(&existing).unwrap_or_default()
                    };
                entries.retain(|e| e.sender_public_key != response.sender_public_key);
                entries.push(response);

                let bytes = serde_json::to_vec(&entries)
                    .map_err(|e| ChatError::Serialization(format!("{e}")))?;
                let _ = self.io
                    .write_record(&req_inbox_key, subkey, &bytes, Some(&kp_bytes), Confirm::Accepted)
                    .await;
            }
        }

        // Remove from pending
        {
            let mut meta = self.session_meta.write();
            meta.remove_pending_friend_request(peer_pubkey);
        }

        tracing::info!(
            peer = &peer_pubkey[..12.min(peer_pubkey.len())],
            "friend request rejected"
        );

        Ok(())
    }
}