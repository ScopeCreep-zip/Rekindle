//! Messaging delegation — DM and channel message operations.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub async fn send_dm(
        &self, peer_key: &str, body: &str,
    ) -> Result<crate::messaging::dm::DmSentResult, ChatError> {
        self.messaging.send_dm(peer_key, body).await
    }

    pub async fn send_channel_message(
        &self, community: &str, channel: &str, body: &str, reply_to: Option<u64>,
    ) -> Result<crate::messaging::channel::ChannelSentResult, ChatError> {
        self.messaging.send_channel_message(community, channel, body, reply_to).await
    }

    pub async fn edit_channel_message(
        &self, community: &str, channel: &str, message_id: &str, new_body: &str,
    ) -> Result<(), ChatError> {
        self.messaging.edit_channel_message(community, channel, message_id, new_body).await
    }

    pub async fn delete_channel_message(
        &self, community: &str, channel: &str, message_id: &str,
    ) -> Result<(), ChatError> {
        self.messaging.delete_channel_message(community, channel, message_id).await
    }

    pub async fn send_channel_typing(
        &self, community: &str, channel: &str,
    ) -> Result<(), ChatError> {
        self.messaging.send_channel_typing(community, channel).await
    }

    pub fn dm_thread(
        &self, peer_key: &str, limit: u32,
    ) -> Result<Vec<rekindle_storage::messages::DmRecord>, ChatError> {
        Ok(self.vault.query_dm_thread(peer_key, limit)?)
    }

    pub fn channel_history(
        &self, community: &str, channel: &str, limit: u32,
    ) -> Result<Vec<rekindle_storage::messages::ChannelRecord>, ChatError> {
        Ok(self.vault.query_channel_history(community, channel, limit)?)
    }

    pub async fn send_dm_typing(&self, peer_key: &str, typing: bool) -> Result<(), ChatError> {
        let payload = if typing {
            rekindle_types::dm_payload::DmPayload::Typing { typing: true }
        } else {
            rekindle_types::dm_payload::DmPayload::Typing { typing: false }
        };
        self.io.send_peer_notification(peer_key, payload, crate::io::Confirm::None).await?;
        Ok(())
    }

    pub fn dm_inbox(&self, limit: u32) -> Vec<crate::messaging::dm::DmInboxEntry> {
        let meta = self.session_meta.read();
        let mut entries = Vec::new();
        for peer_key in meta.dm_peers.keys() {
            let display_name = meta.friend_display_names
                .get(peer_key)
                .cloned()
                .unwrap_or_else(|| peer_key[..12.min(peer_key.len())].to_string());
            entries.push(crate::messaging::dm::DmInboxEntry {
                peer_key: peer_key.clone(),
                display_name,
            });
        }
        entries.truncate(limit as usize);
        entries
    }
}
