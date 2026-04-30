//! Channel administration operations — create, delete, update.
//!
//! These modify the governance manifest's channels subkey.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dht_types::{ChannelEntry, ChannelKind};

/// Create a new channel in a community.
///
/// Reads the current channel list from governance, validates no name
/// collision, appends the new channel, and writes back.
pub async fn create_channel(
    node: &TransportNode,
    governance_key: &str,
    name: &str,
    kind: &str,
    category_id: Option<&str>,
    topic: Option<&str>,
    slowmode_seconds: u32,
) -> Result<ChannelEntry> {
    let dht = node.dht()?;
    let mut channels = dht.governance().read_channels(governance_key).await?;

    // Validate no name collision within the same category
    let same_category = category_id.map(String::from);
    if channels
        .iter()
        .any(|ch| ch.name == name && ch.category_id == same_category)
    {
        return Err(TransportError::DhtError {
            reason: format!("channel '{name}' already exists in this category"),
        });
    }

    let channel_kind = parse_channel_kind(kind)?;
    let channel_id = uuid::Uuid::new_v4().to_string();

    // Sort order: append after last channel in the same category
    let sort_order = channels
        .iter()
        .filter(|ch| ch.category_id == same_category)
        .map(|ch| ch.sort_order)
        .max()
        .map_or(0, |max| max + 1);

    let entry = ChannelEntry {
        id: channel_id,
        name: name.to_string(),
        kind: channel_kind,
        sort_order,
        category_id: category_id.map(String::from),
        topic: topic.unwrap_or_default().to_string(),
        slowmode_seconds,
        nsfw: false,
        message_record_key: None,
        mek_generation: 0,
        log_key: None,
    };

    channels.push(entry.clone());
    dht.governance()
        .write_channels(governance_key, &channels)
        .await?;

    info!(channel = name, governance = governance_key, "channel created");
    Ok(entry)
}

/// Delete a channel by ID.
pub async fn delete_channel(
    node: &TransportNode,
    governance_key: &str,
    channel_id: &str,
) -> Result<()> {
    let dht = node.dht()?;
    let mut channels = dht.governance().read_channels(governance_key).await?;

    let before = channels.len();
    channels.retain(|ch| ch.id != channel_id);

    if channels.len() == before {
        return Err(TransportError::DhtError {
            reason: format!("channel '{channel_id}' not found"),
        });
    }

    dht.governance()
        .write_channels(governance_key, &channels)
        .await?;

    info!(channel = channel_id, "channel deleted");
    Ok(())
}

/// Update a channel's properties.
///
/// Only updates fields that are `Some`.
pub async fn update_channel(
    node: &TransportNode,
    governance_key: &str,
    channel_id: &str,
    name: Option<&str>,
    topic: Option<&str>,
    slowmode_seconds: Option<u32>,
) -> Result<ChannelEntry> {
    let dht = node.dht()?;
    let mut channels = dht.governance().read_channels(governance_key).await?;

    let channel = channels
        .iter_mut()
        .find(|ch| ch.id == channel_id)
        .ok_or_else(|| TransportError::DhtError {
            reason: format!("channel '{channel_id}' not found"),
        })?;

    if let Some(n) = name {
        channel.name = n.to_string();
    }
    if let Some(t) = topic {
        channel.topic = t.to_string();
    }
    if let Some(s) = slowmode_seconds {
        channel.slowmode_seconds = s;
    }

    let updated = channel.clone();
    dht.governance()
        .write_channels(governance_key, &channels)
        .await?;

    info!(channel = channel_id, "channel updated");
    Ok(updated)
}

fn parse_channel_kind(kind: &str) -> Result<ChannelKind> {
    match kind.to_lowercase().as_str() {
        "text" => Ok(ChannelKind::Text),
        "voice" => Ok(ChannelKind::Voice),
        "announcement" => Ok(ChannelKind::Announcement),
        "forum" => Ok(ChannelKind::Forum),
        "stage" => Ok(ChannelKind::Stage),
        "directory" => Ok(ChannelKind::Directory),
        "media" => Ok(ChannelKind::Media),
        "events" => Ok(ChannelKind::Events),
        unknown => Err(TransportError::DhtError {
            reason: format!(
                "unknown channel kind '{unknown}' — expected: text, voice, announcement, forum, stage, directory, media, events"
            ),
        }),
    }
}
