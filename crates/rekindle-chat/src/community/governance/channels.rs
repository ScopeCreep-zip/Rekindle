//! Channel management — create, delete, update, register member channel records.
//!
//! Channels are stored in the governance manifest MANIFEST_CHANNELS subkey.
//! Each channel has a kind (Text, Voice, Announcement, Forum, Stage, Directory,
//! Media, Events), a sort order, optional category, optional topic, and optional
//! slowmode. Channel creation does NOT allocate DhtLog records — that happens
//! per-member when they join the community and register their channel records.

use rekindle_types::dht_types::{
    ChannelEntry, ChannelKind, MANIFEST_CHANNELS, REGISTRY_MEMBER_INDEX,
};

use crate::io::Confirm;
use crate::ChatError;
use super::super::CommunityService;

impl CommunityService {
    pub async fn create_channel(
        &self, gov_key: &str, name: &str, kind: &str,
    ) -> Result<ChannelEntry, ChatError> {
        let keypair = self.require_governance_keypair(gov_key)?;
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
            message_record_key: None,
            mek_generation: 0,
            log_key: None,
        };
        channels.push(channel.clone());
        let bytes = serde_json::to_vec(&channels).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(gov_key, MANIFEST_CHANNELS, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_governance_updated(gov_key, MANIFEST_CHANNELS).await;
        Ok(channel)
    }

    pub async fn delete_channel(&self, gov_key: &str, channel_id: &str) -> Result<(), ChatError> {
        let keypair = self.require_governance_keypair(gov_key)?;
        let mut channels = self.read_channels(gov_key).await?;
        channels.retain(|ch| ch.id != channel_id);
        let bytes = serde_json::to_vec(&channels).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(gov_key, MANIFEST_CHANNELS, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_governance_updated(gov_key, MANIFEST_CHANNELS).await;
        Ok(())
    }

    pub async fn update_channel(
        &self, gov_key: &str, channel_id: &str, name: Option<&str>, topic: Option<&str>,
    ) -> Result<ChannelEntry, ChatError> {
        let keypair = self.require_governance_keypair(gov_key)?;
        let mut channels = self.read_channels(gov_key).await?;
        let ch = channels.iter_mut().find(|c| c.id == channel_id)
            .ok_or_else(|| ChatError::ChannelNotFound {
                community: gov_key.into(), channel: channel_id.into(),
            })?;
        if let Some(n) = name { ch.name = n.to_string(); }
        if let Some(t) = topic { ch.topic = t.to_string(); }
        let updated = ch.clone();
        let bytes = serde_json::to_vec(&channels).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(gov_key, MANIFEST_CHANNELS, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_governance_updated(gov_key, MANIFEST_CHANNELS).await;
        Ok(updated)
    }

    pub async fn register_channel_record(
        &self, gov_key: &str, member_pseudonym: &str, channel_id: &str, record_key: &str,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;
        let mut members = self.read_members(&membership.registry_key).await?;
        if let Some(m) = members.iter_mut().find(|m| m.pseudonym_key == member_pseudonym) {
            m.channel_records.insert(channel_id.to_string(), record_key.to_string());
            let bytes = serde_json::to_vec(&members).map_err(|e| ChatError::Serialization(format!("{e}")))?;
            self.io.write_record(&membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), Confirm::Accepted).await?;
        }
        Ok(())
    }
}
