//! Channel listing and decrypted channel history.

use crate::error::Result;
use crate::payload::dht_types::ChannelMessage;

use super::display_map::{channel_to_display, decrypt_channel_body};
use super::{ChannelOverviewDisplay, DecryptedMessageDisplay, QueryEngine};

impl QueryEngine {
    // ── Channel queries ─────────────────────────────────────────────

    /// List channels in a community.
    pub async fn list_channels(&self, governance_key: &str) -> Result<Vec<ChannelOverviewDisplay>> {
        let channels = self.dht.governance().read_channels(governance_key).await?;
        Ok(channels.iter().map(channel_to_display).collect())
    }

    /// Read channel message history with MEK decryption.
    ///
    /// Per-member DhtLog architecture: each member owns their own
    /// append-only DhtLog per channel. This method scans the member
    /// registry for channel_records entries, opens each member's DhtLog,
    /// reads the last N messages from each, decrypts with the MEK, and
    /// merges all messages by (lamport_ts, sender_pseudonym) for
    /// deterministic total ordering across high-latency links.
    pub async fn channel_history(
        &self,
        community_id: &str,
        channel_id: &str,
        _channel_log_key: &str,
        registry_key: &str,
        limit: usize,
        local_channel_record_keys: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DecryptedMessageDisplay>> {
        // Read member index with force_refresh=true to get the latest
        // channel_records entries (RegisterChannelRecord may have just completed).
        let members: Vec<crate::payload::dht_types::MemberSummary> =
            match crate::broadcast::dht::record::get(
                self.dht.routing_context(),
                registry_key,
                crate::payload::dht_types::REGISTRY_MEMBER_INDEX,
                true,
            )
            .await
            {
                Ok(Some(data)) => serde_json::from_slice(&data).unwrap_or_default(),
                _ => Vec::new(),
            };

        // Collect all known DhtLog keys: from registry + from local session.
        // Local session has our own channel_record_keys that may not have
        // propagated to the registry yet (RegisterChannelRecord takes time).
        let mut log_keys_to_scan: Vec<(String, String)> = Vec::new(); // (display_name, log_key)

        for member in &members {
            if let Some(log_key) = member.channel_records.get(channel_id) {
                log_keys_to_scan.push((member.display_name.clone(), log_key.clone()));
            }
        }

        // Add our own local record key if not already in the registry list
        if let Some(local_key) = local_channel_record_keys.get(channel_id) {
            if !log_keys_to_scan.iter().any(|(_, k)| k == local_key) {
                log_keys_to_scan.push(("me".to_string(), local_key.clone()));
            }
        }

        tracing::info!(
            channel_id,
            registry_members = members.len(),
            log_keys_count = log_keys_to_scan.len(),
            local_keys = local_channel_record_keys.len(),
            "channel_history: scanning DhtLogs"
        );

        // Collect raw messages from each member's DhtLog.
        let mut raw_messages: Vec<(String, ChannelMessage)> = Vec::new();

        for (display_name, log_key) in &log_keys_to_scan {
            let log = match crate::broadcast::dht::channel_log::DhtLog::open_read(
                self.dht.routing_context(),
                log_key,
            )
            .await
            {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(
                        member = %display_name, log = %log_key,
                        error = %e, "channel_history: cannot open DhtLog"
                    );
                    continue;
                }
            };

            // Read the last `limit` entries from this member's log
            let entries = match log.tail(u32::try_from(limit).unwrap_or(u32::MAX)).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!(
                        member = %display_name, error = %e,
                        "DhtLog tail read failed"
                    );
                    continue;
                }
            };

            for raw in &entries {
                let msg: ChannelMessage = match serde_json::from_slice(raw) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "skipping malformed DhtLog entry");
                        continue;
                    }
                };
                raw_messages.push((display_name.clone(), msg));
            }
        }

        // Decrypt — lock scoped to this block, no awaits
        let mut messages = Vec::with_capacity(raw_messages.len());
        {
            let mek_cache = self.mek_cache.read();
            for (author_name, channel_msg) in &raw_messages {
                let message_id = channel_msg
                    .message_id
                    .clone()
                    .unwrap_or_else(|| format!("seq:{}", channel_msg.sequence));

                let (body, is_encrypted, needs_mek) =
                    decrypt_channel_body(&mek_cache, community_id, channel_id, channel_msg);

                messages.push(DecryptedMessageDisplay {
                    message_id,
                    sequence: channel_msg.sequence,
                    author_pseudonym: channel_msg.sender_pseudonym.clone(),
                    author_display_name: author_name.clone(),
                    body,
                    timestamp: channel_msg.timestamp,
                    reply_to_sequence: channel_msg.reply_to,
                    mek_generation: channel_msg.mek_generation,
                    is_encrypted,
                    needs_mek,
                });
            }
        }

        // Deterministic total ordering: Lamport timestamp, then sender pseudonym
        messages.sort_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then_with(|| a.author_pseudonym.cmp(&b.author_pseudonym))
        });

        // Return last N messages
        if messages.len() > limit {
            messages = messages.split_off(messages.len() - limit);
        }

        Ok(messages)
    }
}
