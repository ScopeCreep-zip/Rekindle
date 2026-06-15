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
    /// 2. Build ChannelMessage JSON with pseudonym, sequence, ciphertext
    /// 3. Write to the shared SMPL channel record at our slot_index
    /// 4. Persist decrypted plaintext to vault for local history
    /// 5. Broadcast ChannelMessage gossip with ciphertext to mesh peers
    pub async fn send_channel_message(
        &self,
        community: &str,
        channel: &str,
        body: &str,
        reply_to: Option<u64>,
    ) -> Result<ChannelSentResult, ChatError> {
        // Resolve community reference (name or governance key) → typed key + membership.
        let (gov_key, membership) = {
            let meta = self.session_meta.read();
            let (_, m) = meta.resolve_community(community)
                .ok_or_else(|| ChatError::NotMember { community: community.into() })?;
            (m.governance_key.clone(), m.clone())
        };
        let community = &gov_key;

        // Lockdown enforcement
        if membership.locked_down && !membership.is_operator {
            return Err(ChatError::InsufficientPermissions {
                action: format!(
                    "send message to {channel} — community is locked down by operator. \
                     Contact a community operator to lift the lockdown."
                ),
            });
        }
        let channel_id = membership.resolve_channel(channel)
            .map_err(|e| ChatError::ChannelNotFound {
                community: community.into(),
                channel: e.to_string(),
            })?;

        // Slowmode enforcement: check per-(community, channel, pseudonym) rate limit
        let pseudonym_for_slowmode = self.io.pseudonym_hex(community)?;
        let slowmode_key = (community.to_string(), channel_id.clone(), pseudonym_for_slowmode.clone());
        if let Some(slowmode_secs) = membership.channel_slowmode.get(&channel_id).copied().filter(|&s| s > 0) {
            let now = timestamp_ms();
            let window_ms = u64::from(slowmode_secs) * 1000;
            let mut last_send = self.slowmode_last_send.lock();
            if let Some(&last_ts) = last_send.get(&slowmode_key) {
                let elapsed = now.saturating_sub(last_ts);
                if elapsed < window_ms {
                    let remaining = (window_ms - elapsed) / 1000 + 1;
                    return Err(ChatError::SlowmodeActive { remaining_secs: remaining });
                }
            }
            last_send.insert(slowmode_key.clone(), now);
        }

        let (mek_key, mek_generation) = self.mek_cache
            .current(community, &channel_id)
            .ok_or_else(|| ChatError::MekNotCached {
                community: community.into(),
                channel: channel_id.clone(),
            })?;

        let message_id = uuid::Uuid::new_v4().to_string();
        let timestamp = timestamp_ms();
        let pseudonym_hex = self.io.pseudonym_hex(community)?;

        // Increment sequence + lamport BEFORE encrypt — AAD binds to sequence.
        let (sequence, lamport_ts) = {
            let mut meta = self.session_meta.write();
            let m = meta.communities.get_mut(community)
                .ok_or_else(|| ChatError::NotMember { community: community.into() })?;
            let seq_key = format!("self:{channel_id}");
            let seq = m.last_seen_seqs.entry(seq_key).or_insert(0);
            *seq += 1;
            m.lamport_counter += 1;
            (*seq, m.lamport_counter)
        };

        let subkey = membership.slot_index;
        let channel_key_ref = membership.channel_record_keys.get(&channel_id)
            .map(String::as_str).unwrap_or("");
        let aad = build_channel_aad(channel_key_ref, subkey, sequence);
        let encrypted = mek::mek_encrypt(&mek_key, body.as_bytes(), &aad)?;

        let entry = serde_json::json!({
            "sequence": sequence,
            "sender_pseudonym": pseudonym_hex,
            "ciphertext": hex::encode(&encrypted),
            "mek_generation": mek_generation,
            "timestamp": timestamp,
            "message_id": message_id,
            "reply_to": reply_to,
            "lamport_ts": lamport_ts,
        });
        let entry_bytes = serde_json::to_vec(&entry)
            .map_err(|e| ChatError::Serialization(format!("channel entry: {e}")))?;

        // channel_record_keys maps channel_id → shared SMPL record key
        let channel_key = membership.channel_record_keys.get(&channel_id)
            .ok_or_else(|| ChatError::ChannelNotFound {
                community: community.into(),
                channel: channel_id.clone(),
            })?;

        // Derive slot keypair from shared slot_seed for SMPL write
        let slot_seed = membership.slot_seed.as_ref()
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ChatError::Internal("slot_seed not available for channel write".into()))?;

        let slot_kp = rekindle_identity::derive_slot_keypair(&slot_seed, membership.slot_index)
            .map_err(|e| ChatError::Internal(format!("slot keypair: {e}")))?;
        let slot_kp_bytes = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&slot_kp.public_key_bytes());
            let mut ikm = Vec::with_capacity(36);
            ikm.extend_from_slice(&slot_seed);
            ikm.extend_from_slice(&membership.slot_index.to_le_bytes());
            let seed = blake3::derive_key(
                rekindle_identity::derivation_tags::SLOT_KEYPAIR,
                &ikm,
            );
            buf[32..].copy_from_slice(&seed);
            buf
        };

        if let Err(e) = self.io.open_and_write(
            channel_key, subkey, &entry_bytes, Some(&slot_kp_bytes), Confirm::Accepted,
        ).await {
            tracing::warn!(error = %e, "channel write failed — queuing for retry");
            let _ = self.retry_tx.try_send(super::PendingChannelWrite {
                channel_key: channel_key.to_string(),
                subkey,
                data: entry_bytes.clone(),
                writer: slot_kp_bytes.to_vec(),
                attempt: 0,
            });
        }

        tracing::info!(
            community = &community[..20.min(community.len())],
            channel_id = &channel_id[..20.min(channel_id.len())],
            message_id = &message_id[..12.min(message_id.len())],
            "store_channel_message: persisting to vault"
        );
        self.vault.store_channel_message(
            community, &channel_id, &pseudonym_hex, "", body, timestamp, sequence,
            &message_id, mek_generation, reply_to, None,
        )?;

        // Gossip: broadcast encrypted message to mesh peers via dedup path.
        // Carries the full ciphertext so receivers can MEK-decrypt immediately
        // without reading the sender's DhtLog. The DhtLog write above is the
        // durable slow path; gossip is the sub-second fast path.
        let gossip = GossipPayload::ChannelMessage {
            channel_id: channel_id.clone(),
            message_id: message_id.clone(),
            ciphertext: encrypted,
            mek_generation,
            sequence,
            timestamp,
            reply_to,
            thread_id: None,
        };

        match self.io.broadcast_gossip_dedup(community, gossip).await {
            Ok(receipt) => {
                tracing::info!(
                    community = &community[..20.min(community.len())],
                    channel_id = &channel_id[..12.min(channel_id.len())],
                    message_id = &message_id[..12.min(message_id.len())],
                    peers_sent = receipt.peers_sent,
                    peers_failed = receipt.peers_failed,
                    elapsed_ms = receipt.elapsed.as_millis(),
                    "channel send: gossip broadcast complete"
                );
            }
            Err(e) => {
                tracing::warn!(
                    community = &community[..12.min(community.len())],
                    error = %e,
                    "channel send: gossip broadcast FAILED — slow-path catch-up is fallback"
                );
            }
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

        let new_ciphertext = mek::mek_encrypt(&mek_key, new_body.as_bytes(), &[])?;

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

}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelSentResult {
    pub message_id: String,
    pub timestamp: u64,
}

/// Build AAD bytes for channel message encryption: `channel_key ‖ subkey(4 LE) ‖ sequence(8 LE)`.
/// Binds ciphertext to a specific channel record, member slot, and sequence position.
/// A ciphertext encrypted with this AAD cannot be replayed to a different channel,
/// impersonated from a different slot, or replayed at a different sequence.
pub fn build_channel_aad(channel_key: &str, subkey: u32, sequence: u64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(channel_key.len() + 12);
    aad.extend_from_slice(channel_key.as_bytes());
    aad.extend_from_slice(&subkey.to_le_bytes());
    aad.extend_from_slice(&sequence.to_le_bytes());
    aad
}