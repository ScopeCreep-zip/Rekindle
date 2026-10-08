//! Channel listing and decrypted channel history.

use std::collections::HashMap;

use crate::error::Result;
use crate::payload::dht_types::ChannelMessage;

use super::display_map::decrypt_channel_body;
use super::{DecryptedMessageDisplay, QueryEngine};

impl QueryEngine {
    // ── Channel queries ─────────────────────────────────────────────

    /// Read channel message history with MEK decryption.
    ///
    /// SMPL segment-record architecture: one record per `(channel,
    /// segment)`, every member writing to their own slot subkey.
    /// `record_keys` is `(segment_index, record_key)` as merged
    /// governance reports it, `writers` is `(segment_index, slot)` per
    /// known member (the roster plus ourselves: only those slots are read,
    /// plan C7.12), and `display_names` maps a pseudonym to the name the
    /// presence roster observed for it.
    ///
    /// This used to read the registry's member-index subkey to learn
    /// each member's `DhtLog` spine key. That subkey is a community-wide
    /// aggregate no `o_cnt: 0` writer is credentialed for, and it was
    /// the last thing keeping the index alive on this track.
    pub async fn channel_history(
        &self,
        community_id: &str,
        channel_id: &str,
        record_keys: &[(u32, String)],
        writers: &[(u32, u32)],
        display_names: &HashMap<String, String>,
        limit: usize,
        keys: &dyn rekindle_types::channel_keys::ChannelKeyProvider,
    ) -> Result<Vec<DecryptedMessageDisplay>> {
        // Channel text is under the channel's text scope (plan D6).
        let scope = rekindle_types::id::ChannelId::from_hex(channel_id)
            .map(|channel| keys.scope_for_text(community_id, channel))
            .ok_or_else(|| {
                crate::error::TransportError::Internal(format!("not a channel id: {channel_id}"))
            })?;
        // (record key, subkey, message): the body's AAD position.
        let mut raw_messages: Vec<(String, u32, ChannelMessage)> = Vec::new();

        for (segment_index, record_key) in record_keys {
            // The segment's writers, from the roster (plan C7.12).
            let slots: Vec<u32> = writers
                .iter()
                .filter(|(segment, _)| segment == segment_index)
                .map(|(_, slot)| *slot)
                .collect();
            match crate::broadcast::dht::channel_smpl::read_message_items(
                &self.records,
                record_key,
                &slots,
            )
            .await
            {
                Ok(items) => raw_messages.extend(
                    items
                        .into_iter()
                        .map(|(subkey, message)| (record_key.clone(), subkey, message)),
                ),
                Err(error) => {
                    // One unreadable segment must not blank the whole
                    // channel — the other segments still hold messages.
                    tracing::warn!(
                        channel_id,
                        segment = segment_index,
                        record = %record_key,
                        %error,
                        "channel_history: segment record unreadable, skipping"
                    );
                }
            }
        }

        tracing::debug!(
            channel_id,
            segments = record_keys.len(),
            messages = raw_messages.len(),
            "channel_history: scanned channel segment records"
        );

        let mut messages = Vec::with_capacity(raw_messages.len());
        {
            for (record_key, subkey_index, channel_msg) in &raw_messages {
                let message_id = channel_msg
                    .message_id
                    .clone()
                    .unwrap_or_else(|| format!("seq:{}", channel_msg.sequence));

                let (body, is_encrypted, needs_mek) = decrypt_channel_body(
                    keys,
                    community_id,
                    scope,
                    record_key,
                    *subkey_index,
                    channel_msg,
                );

                messages.push(DecryptedMessageDisplay {
                    message_id,
                    sequence: channel_msg.sequence,
                    author_pseudonym: channel_msg.sender_pseudonym.clone(),
                    // The roster is the only name source now. A message
                    // from someone who has since left has no row to name
                    // them, so it falls back to their pseudonym rather
                    // than rendering blank.
                    author_display_name: display_names
                        .get(&channel_msg.sender_pseudonym)
                        .cloned()
                        .unwrap_or_else(|| channel_msg.sender_pseudonym.clone()),
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
