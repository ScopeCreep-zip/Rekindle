//! Messaging — DM and channel message operations.

pub mod dm;
pub mod channel;

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_storage::VaultStore;
use rekindle_types::session_types::SessionMeta;
use rekindle_types::subscription_events::{SubscriptionEvent, ChannelMessageEvent};

use crate::crypto::sessions::SessionCache;
use crate::crypto::mek::MekCache;
use crate::events::pipeline::EventPipeline;
use crate::io::PlatformIO;
use crate::ChatError;

pub struct MessagingService {
    pub(crate) io: Arc<PlatformIO>,
    pub(crate) vault: Arc<VaultStore>,
    pub(crate) session_meta: Arc<RwLock<SessionMeta>>,
    pub(crate) session_cache: Arc<SessionCache>,
    pub(crate) mek_cache: Arc<MekCache>,
    pub(crate) pipeline: Arc<EventPipeline>,
}

impl MessagingService {
    /// Handle an inbound typing indicator.
    ///
    /// Validates the sender is a known DM peer before the event reaches
    /// the pipeline. At 100K+ agents, typing indicator spam from unknown
    /// senders is a real attack vector — gate it here.
    ///
    /// Returns `true` if the sender is a known peer and the event should
    /// propagate through the pipeline. Returns `false` if the sender is
    /// unknown and the event should be dropped.
    pub fn handle_typing(&self, sender_key: &str, _payload: &[u8]) -> bool {
        let known = self.session_meta.read().dm_peers.contains_key(sender_key);
        if !known {
            tracing::debug!(
                sender = &sender_key[..12.min(sender_key.len())],
                "typing indicator from unknown peer — dropping"
            );
        }
        known
    }

    /// Handle an inbound DM presence update.
    ///
    /// Validates the sender is a known DM peer before the event reaches
    /// the pipeline. Presence updates from unknown senders are dropped.
    pub fn handle_presence_update(&self, sender_key: &str, _payload: &[u8]) -> bool {
        let known = self.session_meta.read().dm_peers.contains_key(sender_key);
        if !known {
            tracing::debug!(
                sender = &sender_key[..12.min(sender_key.len())],
                "presence update from unknown peer — dropping"
            );
        }
        known
    }

    /// Handle a DM DhtLog value change (watch or poll discovered new entry).
    /// Reads the entry, decrypts via Triple Ratchet, persists to vault,
    /// emits DirectMessageReceived through the event pipeline.
    pub async fn handle_dm_log_change(
        &self,
        peer_key: &str,
        record_key: &str,
        data: Option<Vec<u8>>,
    ) {
        let raw = match data {
            Some(d) => d,
            None => {
                match self.io.read_record(record_key, 0, true).await {
                    Ok(Some(d)) => d,
                    Ok(None) => return,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            peer = &peer_key[..12.min(peer_key.len())],
                            "DM log read FAILED — message may be missed until next poll"
                        );
                        return;
                    }
                }
            }
        };
        self.process_dm_entry(peer_key, &raw).await;
    }

    /// Handle a channel DhtLog value change (watch or poll discovered new entry).
    /// Reads the entry, MEK-decrypts, persists to vault, emits
    /// ChannelMessage::New through the event pipeline.
    pub fn handle_channel_log_change(
        &self,
        community: &str,
        channel_id: &str,
        member: &str,
        data: Option<Vec<u8>>,
    ) {
        let Some(raw) = data else {
            tracing::debug!(
                community = &community[..12.min(community.len())],
                channel_id,
                member = &member[..12.min(member.len())],
                "channel log change with no data — skipping (will retry on next poll)"
            );
            return;
        };

        match self.process_channel_entry(community, channel_id, member, &raw) {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(
                    community = &community[..12.min(community.len())],
                    channel_id,
                    member = &member[..12.min(member.len())],
                    error = %e,
                    "channel message processing FAILED — message may appear on next poll"
                );
            }
        }
    }

    /// Process a raw DM DhtLog entry: parse JSON, hex-decode ciphertext,
    /// load session, Triple Ratchet decrypt, persist plaintext, emit event.
    async fn process_dm_entry(&self, peer_key: &str, raw: &[u8]) {
        let entry: serde_json::Value = match serde_json::from_slice(raw) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    peer = &peer_key[..12.min(peer_key.len())],
                    "DM entry JSON parse FAILED — raw bytes may be corrupted"
                );
                return;
            }
        };

        let body_hex = entry.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let timestamp = entry.get("timestamp").and_then(serde_json::Value::as_u64).unwrap_or(0);

        let ciphertext = match hex::decode(body_hex) {
            Ok(ct) => ct,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    peer = &peer_key[..12.min(peer_key.len())],
                    "DM body hex decode FAILED — entry may be malformed"
                );
                return;
            }
        };

        let session_id = match self.session_cache.ensure_loaded(peer_key).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    peer = &peer_key[..12.min(peer_key.len())],
                    "no Signal session for DM decrypt — message unreadable. \
                     Friend may need to re-establish the session."
                );
                return;
            }
        };

        let decrypt_result = self.session_cache.with_session(&session_id, |session| {
            let skipped = VaultSkippedCallback {
                vault: &self.vault,
                session_id: &session_id,
            };
            rekindle_ratchet::ratchet::triple::decrypt(session, &[], &ciphertext, &skipped)
                .map_err(ChatError::from)
        }).await;

        match decrypt_result {
            Ok(plaintext) => {
                let body = String::from_utf8(plaintext).unwrap_or_else(|_| "[binary]".into());
                let sender_name = self.session_meta.read()
                    .friend_display_names
                    .get(peer_key)
                    .cloned()
                    .unwrap_or_else(|| peer_key[..12.min(peer_key.len())].to_string());

                let message_id = format!("dm-{}", uuid::Uuid::now_v7());
                if let Err(e) = self.vault.store_received_dm(
                    peer_key, &sender_name, &body, timestamp, 0, &message_id,
                ) {
                    tracing::error!(
                        error = %e,
                        peer = &peer_key[..12.min(peer_key.len())],
                        "DM vault persist FAILED — message decrypted but not saved"
                    );
                }

                // Emit through pipeline so clients see the message
                self.pipeline.process(SubscriptionEvent::ChannelMessage(
                    ChannelMessageEvent::DirectMessageReceived {
                        peer_key: peer_key.into(),
                        timestamp,
                        sender_name: Some(sender_name),
                        body: Some(body),
                        is_self: false,
                    },
                ));

                tracing::info!(
                    peer = &peer_key[..12.min(peer_key.len())],
                    "DM received, decrypted, persisted, emitted"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    peer = &peer_key[..12.min(peer_key.len())],
                    "DM decrypt FAILED — message unreadable. \
                     Possible causes: ratchet desync, session corruption, \
                     or message from an unknown session. Consider session reset."
                );
            }
        }
    }

    /// Process a raw channel DhtLog entry: parse JSON, hex-decode ciphertext,
    /// MEK-decrypt, persist plaintext, emit event.
    fn process_channel_entry(
        &self,
        community: &str,
        channel_id: &str,
        member_pseudonym: &str,
        raw: &[u8],
    ) -> Result<(), ChatError> {
        let entry: serde_json::Value = serde_json::from_slice(raw)
            .map_err(|e| ChatError::Deserialization(format!("channel entry: {e}")))?;

        let ct_hex = entry.get("ciphertext").and_then(|v| v.as_str()).unwrap_or("");
        let mek_gen = entry.get("mek_generation").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let timestamp = entry.get("timestamp").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let message_id = entry.get("message_id").and_then(serde_json::Value::as_str).unwrap_or("");
        let sequence = entry.get("sequence").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let sender = entry.get("sender_pseudonym").and_then(|v| v.as_str()).unwrap_or(member_pseudonym);

        let ciphertext = hex::decode(ct_hex)
            .map_err(|e| ChatError::Deserialization(format!("channel ciphertext hex: {e}")))?;

        let mek_key = self.mek_cache.get_generation(community, channel_id, mek_gen)
            .ok_or_else(|| {
                // Auto-request the missing MEK from the operator
                tracing::info!(
                    community = &community[..12.min(community.len())],
                    channel = channel_id,
                    generation = mek_gen,
                    "MEK not cached — message unreadable until MEK received"
                );
                ChatError::MekNotCached {
                    community: community.into(),
                    channel: channel_id.into(),
                }
            })?;

        let plaintext_bytes = crate::crypto::mek::mek_decrypt(&mek_key, &ciphertext)?;
        let body = String::from_utf8(plaintext_bytes)
            .unwrap_or_else(|_| "[binary content]".into());

        self.vault.store_channel_message(
            community, channel_id, sender, "",
            &body, timestamp, sequence, message_id, mek_gen,
        )?;

        // Emit through pipeline so clients see the message
        self.pipeline.process(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::New {
                community: community.into(),
                channel: channel_id.into(),
                message_id: message_id.into(),
                sender_pseudonym: sender.into(),
                sequence,
                timestamp,
                body: Some(body),
                reply_to_sequence: None,
                is_self: false,
                client_msg_id: None,
            },
        ));

        tracing::info!(
            community = &community[..12.min(community.len())],
            channel = channel_id,
            sender = &sender[..12.min(sender.len())],
            "channel message received, decrypted, persisted, emitted"
        );

        Ok(())
    }
}

/// Adapter connecting rekindle-ratchet's SkippedKeyCallback to rekindle-storage.
///
/// Carries the actual session_id so skipped keys are stored and retrieved
/// for the correct session. Using a wrong session_id causes permanent
/// loss of skipped message keys — those messages become unrecoverable.
struct VaultSkippedCallback<'a> {
    vault: &'a VaultStore,
    session_id: &'a [u8; 32],
}

impl rekindle_ratchet::ratchet::ec::SkippedKeyCallback for VaultSkippedCallback<'_> {
    fn store_skipped(
        &self,
        header_key: &[u8; 32],
        counter: u32,
        message_key: &zeroize::Zeroizing<[u8; 32]>,
    ) -> Result<(), rekindle_ratchet::RatchetError> {
        self.vault
            .store_skipped_key(self.session_id, header_key, counter, message_key)
            .map_err(|e| rekindle_ratchet::RatchetError::SessionCorrupt(format!("skipped key store: {e}")))
    }

    fn take_skipped(
        &self,
        header_key: &[u8; 32],
        counter: u32,
    ) -> Result<Option<zeroize::Zeroizing<[u8; 32]>>, rekindle_ratchet::RatchetError> {
        self.vault
            .take_skipped_key(self.session_id, header_key, counter)
            .map_err(|e| rekindle_ratchet::RatchetError::SessionCorrupt(format!("skipped key take: {e}")))
    }
}
