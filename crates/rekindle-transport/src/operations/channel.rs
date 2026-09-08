//! Channel message operations — send.
//!
//! Writes into the channel's SMPL segment record via
//! `broadcast::dht::channel_smpl`. MEK encryption is business logic
//! here; the Veilid keypair is parsed inside the broadcast boundary.

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
    pub timestamp: u64,
    pub channel_record_key: String,
    /// BLAKE3 of the ciphertext exactly as written to the record.
    ///
    /// Returned so the caller's gossip notification can carry it: a
    /// peer that fetches the subkey verifies what it read against this,
    /// which is what makes the notification safe to act on without
    /// trusting the forwarding hops.
    pub content_hash: String,
    /// The sequence written into the record. The daemon keeps no
    /// per-channel counter, so this is 0 — reported rather than hidden
    /// so the gossip notification and the record agree.
    pub sequence: u64,
}

/// Where a channel write lands: the segment record and our slot in it.
///
/// The caller resolves this — the record key comes from merged
/// governance (`ChannelCreated` for segment 0, `ChannelSegmentLinked`
/// beyond it) and the keypair is derived from the shared slot seed, so
/// neither is transport's to invent.
#[derive(Debug, Clone)]
pub struct ChannelWriteTarget {
    pub channel_record_key: String,
    pub slot_index: u32,
    pub slot_keypair_str: String,
}

/// Send a message to a community channel.
pub async fn send_message(
    node: &TransportNode,
    membership: &CommunityMembership,
    channel_id: &str,
    plaintext: &str,
    reply_to_sequence: Option<u64>,
    mek_cache: &Arc<RwLock<MekCache>>,
    target: &ChannelWriteTarget,
    signing_key: &ed25519_dalek::SigningKey,
) -> Result<MessageSent> {
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
        // The daemon send path does not yet offer attachments, message
        // flags or mentions. These serialize away to nothing
        // (skip_serializing_if / default), so the bytes are unchanged
        // from before this crate adopted the desktop track's fuller
        // definition — but a message that DOES carry them now survives
        // the round trip instead of being silently dropped.
        attachment: None,
        flags: 0,
        mentioned_pseudonyms: Vec::new(),
        mentioned_roles: Vec::new(),
    };

    // Step 3: Write to our slot in the channel's segment record.
    let author = rekindle_types::id::PseudonymKey::from_hex_lossy(&membership.pseudonym_key);
    crate::broadcast::dht::channel_smpl::write_message(
        node,
        &target.channel_record_key,
        target.slot_index,
        &target.slot_keypair_str,
        author,
        signing_key,
        &channel_msg,
    )
    .await?;

    info!(
        message_id,
        channel = channel_id,
        community = %membership.community_name,
        record = %target.channel_record_key,
        slot = target.slot_index,
        "message written to channel segment record"
    );

    Ok(MessageSent {
        message_id,
        timestamp,
        channel_record_key: target.channel_record_key.clone(),
        content_hash: blake3::hash(&channel_msg.ciphertext).to_hex().to_string(),
        sequence: channel_msg.sequence,
    })
}
