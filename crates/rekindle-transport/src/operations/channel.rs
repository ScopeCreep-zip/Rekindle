//! Channel message operations — send.
//!
//! Writes into the channel's SMPL segment record via
//! `broadcast::dht::channel_smpl`. MEK encryption is business logic
//! here; the Veilid keypair is parsed inside the broadcast boundary.

use rekindle_secrets::channel_body::BodyPosition;
use rekindle_types::channel_keys::ChannelKeyProvider;
use rekindle_types::id::ChannelId;
use tracing::info;

use crate::broadcast::node::TransportNode;
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
    /// The message's Lamport timestamp (from the community message clock).
    pub lamport_ts: u64,
    /// The sequence written into the record. The daemon keeps no
    /// per-channel counter, so this is 0 — reported rather than hidden
    /// so the gossip notification and the record agree.
    pub sequence: u64,
    /// Stored at consensus; `false` while the record pool holds the write
    /// for re-push (plan C7.13).
    pub stored: bool,
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

/// Send a message to a community channel. `lamport_ts` comes from the
/// community's message clock (`SubscriptionManager::next_message_lamport`),
/// never the wall clock: desktop peers order and drift-bound by it.
pub async fn send_message(
    node: &TransportNode,
    membership: &CommunityMembership,
    channel_id: &str,
    plaintext: &str,
    reply_to_sequence: Option<u64>,
    lamport_ts: u64,
    keys: &dyn ChannelKeyProvider,
    target: &ChannelWriteTarget,
    signing_key: &ed25519_dalek::SigningKey,
) -> Result<MessageSent> {
    // Step 1: Encrypt under the community key (plan D6: channel text uses
    // the community scope), bound to the record, subkey and Lamport
    // position it is written at — the codec every track shares.
    let subkey_index =
        u32::from(rekindle_types::dht_layout::channel::OWNER_SUBKEY_COUNT) + target.slot_index;
    let missing = || TransportError::MekNotCached {
        community: membership.community_name.clone(),
        channel: channel_id.to_string(),
        generation: 0,
    };
    let scope = ChannelId::from_hex(channel_id)
        .map(|channel| keys.scope_for_text(&membership.governance_key, channel))
        .ok_or_else(missing)?;
    let (epoch, key) =
        rekindle_types::channel_keys::current_key(keys, &membership.governance_key, scope)
            .ok_or_else(missing)?;
    let ciphertext = rekindle_secrets::channel_body::encrypt_channel_body(
        &key,
        BodyPosition {
            channel_record_key: &target.channel_record_key,
            subkey_index,
            lamport_ts,
        },
        plaintext.as_bytes(),
    )?;
    let mek_generation = epoch.0;

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
        lamport_ts,
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
    let written = crate::broadcast::dht::channel_smpl::write_message(
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
        lamport_ts,
        sequence: channel_msg.sequence,
        stored: written == rekindle_protocol::dht::community::channel_record::AppendOutcome::Stored,
    })
}
