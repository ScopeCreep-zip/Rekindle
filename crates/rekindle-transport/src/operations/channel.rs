//! Channel message operations — send, read history.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::{Mek, MekCache};
use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dht_types::ChannelMessage;
use crate::session::CommunityMembership;

/// Result of a successful channel message send.
#[derive(Debug, Clone)]
pub struct MessageSent {
    /// Unique message ID.
    pub message_id: String,
    /// Sequence number in the channel log.
    pub sequence: u64,
    /// Timestamp (ms since epoch).
    pub timestamp: u64,
}

/// Send a message to a community channel.
///
/// Steps:
/// 1. Look up the current MEK for the channel
/// 2. Encrypt the plaintext body with the MEK
/// 3. Build a `ChannelMessage` record
/// 4. Append to the channel's DHT log
/// 5. Return `MessageSent` with the message ID and sequence
///
/// The caller (CLI/TUI) is responsible for broadcasting a `MessageNotification`
/// gossip to the community mesh after this returns. The transport layer
/// does not own the mesh peer list — that's application-layer state.
pub async fn send_message(
    node: &TransportNode,
    membership: &CommunityMembership,
    channel_id: &str,
    channel_log_key: &str,
    plaintext: &str,
    reply_to_sequence: Option<u64>,
    mek_cache: &Arc<RwLock<MekCache>>,
) -> Result<MessageSent> {
    // Step 1: Look up MEK
    let ciphertext = {
        let cache = mek_cache.read();
        let mek = cache
            .current(&membership.governance_key, channel_id)
            .ok_or_else(|| TransportError::MekNotCached {
                community: membership.community_name.clone(),
                channel: channel_id.to_string(),
                generation: 0,
            })?;

        // Step 2: Encrypt
        mek.encrypt(plaintext.as_bytes())?
    };

    let mek_generation = mek_cache
        .read()
        .current(&membership.governance_key, channel_id)
        .map_or(0, Mek::generation);

    // Step 3: Build message record
    let message_id = uuid::Uuid::new_v4().to_string();
    let timestamp = rekindle_utils::timestamp_ms();

    let channel_msg = ChannelMessage {
        sequence: 0, // Sender-maintained counter; CLI increments per-send
        sender_pseudonym: membership.pseudonym_key.clone(),
        ciphertext,
        mek_generation,
        timestamp,
        reply_to: reply_to_sequence,
        lamport_ts: 0, // Caller sets via LamportClock
        message_id: Some(message_id.clone()),
    };

    // Step 4: Append to DHT log
    let dht = node.dht()?;
    let slot_keypair = derive_slot_writer(membership)?;

    dht.channel_log()
        .write_message(channel_log_key, membership.slot_index, &channel_msg, slot_keypair)
        .await?;

    info!(
        message_id,
        channel = channel_id,
        community = %membership.community_name,
        "channel message sent"
    );

    Ok(MessageSent {
        message_id,
        sequence: channel_msg.sequence,
        timestamp,
    })
}

/// Derive the Veilid `KeyPair` for writing to this member's slot in the
/// channel SMPL record. Requires the slot seed from the community membership.
fn derive_slot_writer(membership: &CommunityMembership) -> Result<veilid_core::KeyPair> {
    let seed = membership.slot_seed.as_ref().ok_or_else(|| {
        TransportError::Internal(format!(
            "no slot seed for community '{}' — session state incomplete",
            membership.community_name
        ))
    })?;
    crate::dht::registry::derive_slot_veilid_keypair(seed, membership.slot_index)
}
