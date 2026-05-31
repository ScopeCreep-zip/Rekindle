//! Channel message operations — send, read history.
//!
//! DhtLog creation/append via `broadcast::dht_writes` primitives.
//! MEK encryption is business logic here.
//! Keypair bytes are deserialized inside the broadcast boundary.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::broadcast::node::TransportNode;
use crate::crypto::mek::{Mek, MekCache};
use crate::error::{Result, TransportError};
use crate::payload::dht_types::ChannelMessage;
use crate::session::CommunityMembership;

#[derive(Debug, Clone)]
pub struct MessageSent {
    pub message_id: String,
    pub sequence: u64,
    pub timestamp: u64,
    pub member_record_key: String,
    pub new_log_keypair_bytes: Option<Vec<u8>>,
}

/// Send a message to a community channel.
///
/// `existing_log_keypair_bytes`: optional 64-byte serialized keypair for the
/// member's existing DhtLog. Pass None on first write — a new DhtLog is created.
pub async fn send_message(
    node: &TransportNode,
    membership: &CommunityMembership,
    channel_id: &str,
    plaintext: &str,
    reply_to_sequence: Option<u64>,
    mek_cache: &Arc<RwLock<MekCache>>,
    existing_log_keypair_bytes: Option<&[u8]>,
) -> Result<MessageSent> {
    // Deserialize keypair if provided
    let existing_log_keypair = existing_log_keypair_bytes
        .map(crate::broadcast::node::deserialize_keypair)
        .transpose()?;

    // Step 1: Encrypt with MEK
    let ciphertext = {
        let cache = mek_cache.read();
        let mek = cache.current(&membership.governance_key, channel_id)
            .ok_or_else(|| {
                tracing::error!(governance_key = %membership.governance_key, channel_id, "MEK not found");
                TransportError::MekNotCached {
                    community: membership.community_name.clone(),
                    channel: channel_id.to_string(), generation: 0,
                }
            })?;
        mek.encrypt(plaintext.as_bytes())?
    };
    let mek_generation = mek_cache
        .read()
        .current(&membership.governance_key, channel_id)
        .map_or(0, Mek::generation);

    // Step 2: Build message
    let message_id = uuid::Uuid::new_v4().to_string();
    let timestamp = rekindle_utils::timestamp_ms();
    let channel_msg = ChannelMessage {
        sequence: 0,
        sender_pseudonym: membership.pseudonym_key.clone(),
        ciphertext,
        mek_generation,
        timestamp,
        reply_to: reply_to_sequence,
        lamport_ts: timestamp,
        message_id: Some(message_id.clone()),
    };
    let msg_bytes =
        serde_json::to_vec(&channel_msg).map_err(|e| TransportError::SerializationFailed {
            reason: e.to_string(),
        })?;

    // Step 3: Get or create DhtLog via broadcast primitives
    let mut new_log_keypair_bytes = None;
    let (log, spine_key) = if let (Some(key), Some(kp)) = (
        membership.channel_record_keys.get(channel_id),
        existing_log_keypair,
    ) {
        match crate::broadcast::dht_writes::open_dht_log_write(node, key, kp).await {
            Ok(log) => (log, key.clone()),
            Err(e) => {
                tracing::warn!(key, error = %e, "DhtLog reopen failed, creating new");
                let (log, kp) = crate::broadcast::dht_writes::create_dht_log(node).await?;
                let spine = log.spine_key();
                new_log_keypair_bytes = Some(crate::broadcast::node::serialize_keypair(&kp));
                (log, spine)
            }
        }
    } else {
        let (log, kp) = crate::broadcast::dht_writes::create_dht_log(node).await?;
        let spine = log.spine_key();
        new_log_keypair_bytes = Some(crate::broadcast::node::serialize_keypair(&kp));
        info!(log_key = %spine, channel = channel_id, "created per-member DhtLog");
        (log, spine)
    };

    // Step 4: Append
    let sequence = log.append(&msg_bytes).await?;
    info!(message_id, channel = channel_id, community = %membership.community_name, log_key = %spine_key, sequence, "message appended");

    Ok(MessageSent {
        message_id,
        sequence,
        timestamp,
        member_record_key: spine_key,
        new_log_keypair_bytes,
    })
}
