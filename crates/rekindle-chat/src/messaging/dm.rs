//! DM send operations.

use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::MessagingService;

impl MessagingService {
    /// Send a DM to a friend.
    ///
    /// 1. Load session from cache (or vault)
    /// 2. Triple Ratchet encrypt
    /// 3. Persist ratchet state to vault
    /// 4. Build DhtLog entry with sender_key + hex ciphertext
    /// 5. Write to outbound DhtLog with Confirm::Accepted
    /// 6. Persist sent plaintext for local history
    pub async fn send_dm(
        &self,
        peer_key: &str,
        body: &str,
    ) -> Result<DmSentResult, ChatError> {
        let (outbound_log_key, _inbound_log_key) = {
            let meta = self.session_meta.read();
            let peer_log = meta.dm_peers.get(peer_key).ok_or_else(|| ChatError::NotFriends {
                peer_key: peer_key.into(),
            })?;
            if peer_log.outbound_log_key.is_empty() {
                return Err(ChatError::NoOutboundLog { peer_key: peer_key.into() });
            }
            (peer_log.outbound_log_key.clone(), peer_log.inbound_log_key.clone())
        };

        let session_id = self.session_cache.ensure_loaded(peer_key).await?;

        let encrypted = self.session_cache.with_session(&session_id, |session| {
            rekindle_ratchet::ratchet::triple::encrypt(session, body.as_bytes())
                .map_err(ChatError::from)
        }).await?;

        // Persist session state after ratchet step
        self.session_cache.with_session(&session_id, |session| {
            self.session_cache.persist(&session_id, peer_key, session)
        }).await?;

        // Build DhtLog entry
        let sender_key_hex = self.io.identity_public_key_hex()?;
        let entry = serde_json::json!({
            "sender_key": sender_key_hex,
            "body": hex::encode(&encrypted.ciphertext),
            "timestamp": timestamp_ms(),
            "recipient_key": peer_key,
        });
        let entry_bytes = serde_json::to_vec(&entry)
            .map_err(|e| ChatError::Serialization(format!("DM entry: {e}")))?;

        // Load outbound log keypair from vault
        let log_short = &outbound_log_key[..12.min(outbound_log_key.len())];
        let keypair = self.vault.load_key(
            &rekindle_storage::keys::labels::dm_log_keypair(log_short),
        )?;

        self.io.write_record(
            &outbound_log_key, 0, &entry_bytes, keypair.as_deref(), Confirm::Accepted,
        ).await?;

        let message_id = format!("dm-{}", uuid::Uuid::now_v7());
        let timestamp = timestamp_ms();

        // Persist sent plaintext for local history
        self.vault.store_sent_dm(peer_key, body, timestamp, &message_id)?;

        tracing::info!(
            peer = &peer_key[..12.min(peer_key.len())],
            message_id,
            "DM sent"
        );

        Ok(DmSentResult { message_id, timestamp })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DmSentResult {
    pub message_id: String,
    pub timestamp: u64,
}

/// A DM conversation entry for inbox listing.
/// Contains the peer key and their resolved display name.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DmInboxEntry {
    pub peer_key: String,
    pub display_name: String,
}
