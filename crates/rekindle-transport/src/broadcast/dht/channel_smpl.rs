//! Channel messages in SMPL segment records — the v2.0 storage shape.
//!
//! Replaces the per-member DFLT `DhtLog` this track used to allocate
//! per channel. Both models are multi-writer; the difference is where
//! the writers live and how a reader finds them:
//!
//! | | per-member `DhtLog` | SMPL segment record |
//! |---|---|---|
//! | record per channel | one per *member* | one per *segment* |
//! | writer credential | a fresh owner keypair, kept in the OS keyring | the slot keypair already derived from the shared seed |
//! | discovery | announce the spine key into the registry member index | `ChannelCreated.record_key` and `ChannelSegmentLinked`, straight out of the merged CRDT |
//!
//! The third row is the one that mattered. Announcing into the member
//! index needed a writer credentialed for a community-wide subkey, and
//! `o_cnt: 0` grants nobody one — which is why the daemon routed the
//! announcement through an operator (`RegisterChannelRecord`), a
//! coordinator by another name. Under this model the record key is
//! already governance state that every peer merged, so there is nothing
//! to announce.
//!
//! Writes go to `CHANNEL_OWNER_SUBKEY_COUNT + slot_index` and carry a
//! W26 signature over the entry, so a peer holding any slot keypair
//! still cannot forge another member's authorship.

use rekindle_protocol::dht::community::channel_record;
use rekindle_types::id::PseudonymKey;

use crate::broadcast::node::TransportNode;
use crate::error::{Result, TransportError};
use crate::payload::dht_types::ChannelMessage;

/// Append a message to our own slot in a channel segment record.
pub async fn write_message(
    node: &TransportNode,
    channel_key: &str,
    slot_index: u32,
    writer_keypair_str: &str,
    author_pseudonym: PseudonymKey,
    signing_key: &ed25519_dalek::SigningKey,
    message: &ChannelMessage,
) -> Result<()> {
    let writer = writer_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| TransportError::DhtError {
            reason: format!("invalid slot keypair: {e}"),
        })?;
    let dht = node.dht()?;
    let mgr = rekindle_protocol::dht::DHTManager::new(dht.routing_context().clone());
    channel_record::write_member_message(
        &mgr,
        channel_key,
        slot_index,
        writer,
        author_pseudonym,
        signing_key,
        message,
    )
    .await
    .map_err(|e| TransportError::DhtError {
        reason: format!("channel SMPL write: {e}"),
    })
}

/// Read every member's messages from one channel segment record.
///
/// Returns them sorted by `(lamport_ts, sender_pseudonym)` — the same
/// deterministic total order the desktop reader applies, so two peers
/// render one conversation identically.
pub async fn read_messages(
    rc: &veilid_core::RoutingContext,
    channel_key: &str,
    member_count: u32,
) -> Result<Vec<ChannelMessage>> {
    channel_record::read_all_channel_messages(rc, channel_key, member_count)
        .await
        .map_err(|e| TransportError::DhtError {
            reason: format!("channel SMPL read: {e}"),
        })
}
