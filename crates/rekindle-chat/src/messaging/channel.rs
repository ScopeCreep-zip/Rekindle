//! Channel message operations — send, edit, delete, typing indicator,
//! inbound channel log change processing.

use rekindle_types::gossip_payload::{GossipPayload, ControlPayload};

use crate::crypto::mek;
use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::MessagingService;

impl MessagingService {
    /// Send a message to a community channel.
    ///
    /// 1. MEK-encrypt the plaintext
    /// 2. Build DhtLog entry with pseudonym, sequence, ciphertext
    /// 3. Write to the member's per-channel DhtLog
    /// 4. Persist decrypted plaintext to vault for local history
    /// 5. Broadcast MessageNotification gossip to mesh peers via dedup path
    pub async fn send_channel_message(
        &self,
        community: &str,
        channel: &str,
        body: &str,
        _reply_to: Option<u64>,
    ) -> Result<ChannelSentResult, ChatError> {
        // Lockdown enforcement — non-operators cannot send during lockdown.
        // This is enforced here (send path), not just displayed in the TUI.
        // A malicious client that skips this check can still write to their
        // DhtLog, but legitimate clients reject locally before network I/O.
        {
            let meta = self.session_meta.read();
            if let Some(membership) = meta.communities.get(community) {
                if membership.locked_down && !membership.is_operator {
                    return Err(ChatError::InsufficientPermissions {
                        action: format!(
                            "send message to {channel} — community is locked down by operator. \
                             Contact a community operator to lift the lockdown."
                        ),
                    });
                }
            }
        }

        let (mek_key, mek_generation) = self.mek_cache
            .current(community, channel)
            .ok_or_else(|| ChatError::MekNotCached {
                community: community.into(),
                channel: channel.into(),
            })?;

        let encrypted = mek::mek_encrypt(&mek_key, body.as_bytes())?;

        let message_id = uuid::Uuid::new_v4().to_string();
        let timestamp = timestamp_ms();

        let pseudonym_hex = self.io.pseudonym_hex(community)?;

        let entry = serde_json::json!({
            "sequence": 0,
            "sender_pseudonym": pseudonym_hex,
            "ciphertext": hex::encode(&encrypted),
            "mek_generation": mek_generation,
            "timestamp": timestamp,
            "message_id": message_id,
        });
        let entry_bytes = serde_json::to_vec(&entry)
            .map_err(|e| ChatError::Serialization(format!("channel entry: {e}")))?;

        let membership = {
            let meta = self.session_meta.read();
            meta.communities.get(community).cloned()
                .ok_or_else(|| ChatError::NotMember { community: community.into() })?
        };

        let log_key = membership.channel_record_keys.get(channel)
            .ok_or_else(|| ChatError::ChannelNotFound {
                community: community.into(),
                channel: channel.into(),
            })?;

        let log_short = &log_key[..12.min(log_key.len())];
        let keypair = self.vault.load_key(
            &rekindle_storage::keys::labels::channel_log_keypair(log_short),
        )?;

        self.io.write_record(
            log_key, 0, &entry_bytes, keypair.as_deref(), Confirm::Accepted,
        ).await?;

        self.vault.store_channel_message(
            community, channel, &pseudonym_hex, "", body, timestamp, 0,
            &message_id, mek_generation,
        )?;

        // Gossip: notify mesh peers via dedup path
        let content_hash = hex::encode(&blake3::hash(body.as_bytes()).as_bytes()[..16]);
        let notification = GossipPayload::MessageNotification {
            channel_id: channel.into(),
            message_id: message_id.clone(),
            author_pseudonym: pseudonym_hex,
            subkey_index: 0,
            lamport_ts: 0,
            sequence: 0,
            content_hash,
            timestamp,
        };

        if let Err(e) = self.io.broadcast_gossip_dedup(community, notification).await {
            tracing::debug!(
                community = &community[..12.min(community.len())],
                error = %e,
                "message notification gossip failed — peers will discover via watch/poll"
            );
        }

        Ok(ChannelSentResult { message_id, timestamp })
    }

    /// Edit a channel message. Broadcasts MessageEdited gossip notification.
    pub async fn edit_channel_message(
        &self,
        community: &str,
        channel: &str,
        message_id: &str,
        new_body: &str,
    ) -> Result<(), ChatError> {
        let (mek_key, mek_generation) = self.mek_cache
            .current(community, channel)
            .ok_or_else(|| ChatError::MekNotCached {
                community: community.into(),
                channel: channel.into(),
            })?;

        let new_ciphertext = mek::mek_encrypt(&mek_key, new_body.as_bytes())?;

        let ctrl = ControlPayload::MessageEdited {
            channel_id: channel.into(),
            message_id: message_id.into(),
            new_ciphertext,
            mek_generation,
            edited_at: timestamp_ms(),
        };
        if let Err(e) = self.io.broadcast_gossip_dedup(
            community, GossipPayload::Control(ctrl),
        ).await {
            tracing::debug!(error = %e, "edit gossip failed — peers will discover via watch/poll");
        }

        Ok(())
    }

    /// Delete a channel message. Broadcasts MessageDeleted gossip notification.
    pub async fn delete_channel_message(
        &self,
        community: &str,
        channel: &str,
        message_id: &str,
    ) -> Result<(), ChatError> {
        let ctrl = ControlPayload::MessageDeleted {
            channel_id: channel.into(),
            message_id: message_id.into(),
        };
        if let Err(e) = self.io.broadcast_gossip_dedup(
            community, GossipPayload::Control(ctrl),
        ).await {
            tracing::debug!(error = %e, "delete gossip failed — peers will discover via watch/poll");
        }

        Ok(())
    }

    /// Send a typing indicator for a community channel via gossip broadcast.
    pub async fn send_channel_typing(
        &self,
        community: &str,
        channel: &str,
    ) -> Result<(), ChatError> {
        let pseudonym_hex = self.io.pseudonym_hex(community)?;

        let payload = GossipPayload::TypingIndicator {
            channel_id: channel.into(),
            pseudonym_key: pseudonym_hex,
        };

        if let Err(e) = self.io.broadcast_gossip_dedup(community, payload).await {
            tracing::debug!(error = %e, "typing gossip failed");
        }

        Ok(())
    }

    /// Process an inbound channel log change (DHT watch or poll).
    /// Reads the new entry, MEK-decrypts, persists to vault, emits event.
    pub fn process_channel_log_entry(
        &self,
        community: &str,
        channel_id: &str,
        member_pseudonym: &str,
        raw: &[u8],
    ) -> Result<(), ChatError> {
        let entry: serde_json::Value = serde_json::from_slice(raw)
            .map_err(|e| ChatError::Deserialization(format!("channel entry: {e}")))?;

        let ct_hex = entry.get("ciphertext").and_then(serde_json::Value::as_str).unwrap_or("");
        let mek_gen = entry.get("mek_generation").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let timestamp = entry.get("timestamp").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let message_id = entry.get("message_id").and_then(serde_json::Value::as_str).unwrap_or("");
        let sequence = entry.get("sequence").and_then(serde_json::Value::as_u64).unwrap_or(0);

        let ciphertext = hex::decode(ct_hex)
            .map_err(|e| ChatError::Deserialization(format!("channel ciphertext hex: {e}")))?;

        let mek_key = self.mek_cache.get_generation(community, channel_id, mek_gen)
            .ok_or_else(|| {
                tracing::warn!(
                    community = &community[..12.min(community.len())],
                    channel = channel_id,
                    generation = mek_gen,
                    "MEK not cached for channel message — requesting from operator"
                );
                ChatError::MekNotCached {
                    community: community.into(),
                    channel: channel_id.into(),
                }
            })?;

        let plaintext_bytes = mek::mek_decrypt(&mek_key, &ciphertext)?;
        let body = String::from_utf8(plaintext_bytes)
            .unwrap_or_else(|_| "[binary content]".into());

        self.vault.store_channel_message(
            community, channel_id, member_pseudonym, "",
            &body, timestamp, sequence, message_id, mek_gen,
        )?;

        tracing::info!(
            community = &community[..12.min(community.len())],
            channel = channel_id,
            sender = &member_pseudonym[..12.min(member_pseudonym.len())],
            "channel message received and decrypted"
        );

        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelSentResult {
    pub message_id: String,
    pub timestamp: u64,
}