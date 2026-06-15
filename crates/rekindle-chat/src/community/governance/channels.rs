//! Channel management — create, delete, update.
//!
//! Each channel has a shared SMPL record (message_record_key) created at
//! channel creation time with the same 255 member slot pubkeys as the
//! registry and existing channels. Channel record keys are stored in
//! governance MANIFEST_CHANNELS and discovered by all members.

use rekindle_types::dht_types::{
    ChannelEntry, ChannelKind, MANIFEST_CHANNELS,
    CHANNEL_OWNER_SUBKEY_COUNT, CHANNEL_MEMBER_SUBKEY_COUNT,
};
use rekindle_types::transport::RecordSchema;

use crate::ChatError;
use super::super::CommunityService;

impl CommunityService {
    pub async fn create_channel(
        &self, gov_key: &str, name: &str, kind: &str,
    ) -> Result<ChannelEntry, ChatError> {
        let membership = self.require_operator(gov_key)?;

        // Derive member pubkeys from slot_seed for the new channel's SMPL record
        let slot_seed = membership.slot_seed.as_ref()
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ChatError::Internal("slot_seed not available for channel creation".into()))?;

        let mut member_pubkeys = Vec::with_capacity(255);
        for i in 0..255u32 {
            let kp = rekindle_identity::derive_slot_keypair(&slot_seed, i)
                .map_err(|e| ChatError::Internal(format!("slot keypair {i}: {e}")))?;
            member_pubkeys.push(kp.public_key_bytes());
        }

        // Create shared SMPL channel record
        let (channel_record, _) = self.io
            .create_record(RecordSchema::MultiWriter {
                owner_subkeys: CHANNEL_OWNER_SUBKEY_COUNT,
                member_subkeys: CHANNEL_MEMBER_SUBKEY_COUNT,
                member_keys: member_pubkeys,
            })
            .await?;

        let mut channels = self.read_channels(gov_key).await?;
        let channel_kind = match kind.to_lowercase().as_str() {
            "text" => ChannelKind::Text,
            "voice" => ChannelKind::Voice,
            "announcement" => ChannelKind::Announcement,
            "forum" => ChannelKind::Forum,
            "stage" => ChannelKind::Stage,
            "directory" => ChannelKind::Directory,
            "media" => ChannelKind::Media,
            "events" => ChannelKind::Events,
            other => return Err(ChatError::Internal(format!("unknown channel kind: {other}"))),
        };
        let channel = ChannelEntry {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            kind: channel_kind,
            sort_order: u16::try_from(channels.len()).unwrap_or(u16::MAX),
            category_id: None,
            topic: String::new(),
            slowmode_seconds: 0,
            nsfw: false,
            message_record_key: Some(channel_record.key().to_string()),
            mek_generation: 0,
            log_key: None,
            member_log_index_key: None,
        };
        channels.push(channel.clone());
        let bytes = serde_json::to_vec(&channels).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.write_signed_governance(gov_key, MANIFEST_CHANNELS, &bytes).await?;
        self.notify_governance_updated(gov_key, MANIFEST_CHANNELS).await;

        // Update local session meta with the new channel
        {
            let mut meta = self.session_meta.write();
            if let Some(m) = meta.communities.get_mut(gov_key) {
                m.channel_record_keys.insert(channel.id.clone(), channel_record.key().to_string());
                m.channel_name_to_id.insert(channel.name.clone(), channel.id.clone());
                m.channel_slowmode.insert(channel.id.clone(), 0);
            }
        }

        tracing::info!(
            channel = %name,
            channel_key = &channel_record.key()[..12.min(channel_record.key().len())],
            "channel created with shared SMPL record"
        );
        Ok(channel)
    }

    pub async fn delete_channel(&self, gov_key: &str, channel_id: &str) -> Result<(), ChatError> {
        let _membership = self.require_operator(gov_key)?;
        let mut channels = self.read_channels(gov_key).await?;
        channels.retain(|ch| ch.id != channel_id);
        let bytes = serde_json::to_vec(&channels).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.write_signed_governance(gov_key, MANIFEST_CHANNELS, &bytes).await?;
        self.notify_governance_updated(gov_key, MANIFEST_CHANNELS).await;

        // Remove from local session meta
        {
            let mut meta = self.session_meta.write();
            if let Some(m) = meta.communities.get_mut(gov_key) {
                m.channel_record_keys.remove(channel_id);
                m.channel_name_to_id.retain(|_, v| v != channel_id);
                m.channel_slowmode.remove(channel_id);
            }
        }
        Ok(())
    }

    pub async fn update_channel(
        &self, gov_key: &str, channel_id: &str, name: Option<&str>, topic: Option<&str>,
    ) -> Result<ChannelEntry, ChatError> {
        let _membership = self.require_operator(gov_key)?;
        let mut channels = self.read_channels(gov_key).await?;
        let ch = channels.iter_mut().find(|c| c.id == channel_id)
            .ok_or_else(|| ChatError::ChannelNotFound {
                community: gov_key.into(), channel: channel_id.into(),
            })?;
        if let Some(n) = name { ch.name = n.to_string(); }
        if let Some(t) = topic { ch.topic = t.to_string(); }
        let updated = ch.clone();
        let bytes = serde_json::to_vec(&channels).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.write_signed_governance(gov_key, MANIFEST_CHANNELS, &bytes).await?;
        self.notify_governance_updated(gov_key, MANIFEST_CHANNELS).await;
        Ok(updated)
    }
}
