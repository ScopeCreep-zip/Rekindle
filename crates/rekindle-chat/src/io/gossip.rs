//! Gossip broadcast — two paths for different traffic patterns.
//!
//! ## `broadcast_gossip_dedup` (default for all community operations)
//!
//! Constructs a SignedGossipEnvelope, serializes, frames with TypeId 0x0A.
//! Transport recognizes 0x0A and BLAKE3-hashes the payload bytes for
//! content-addressed deduplication before forwarding to mesh peers.
//! Inbound gossip with TypeId 0x0A is also deduped at transport before
//! reaching chat's EventDedup layer.
//!
//! **Traffic cost:** 1 BLAKE3 hash per peer per inbound copy (~50ns).
//! **Savings:** N-1 duplicate deliveries suppressed per N mesh sources.
//!
//! **Use for:** ALL community operations — messages, governance changes,
//! membership updates, MEK rotations, presence, voice signaling,
//! reactions, pins, threads, events, game servers, system messages.
//! Any event that fans out to a mesh and may arrive from multiple peers.
//!
//! **Do NOT use for:** point-to-point delivery to a single known peer.
//! Use `send_gossip_direct` for that — dedup overhead is wasted when
//! there is exactly one sender.
//!
//! ## `broadcast_gossip_unmanaged` (specialized, use with caution)
//!
//! Passes raw bytes to `transport.broadcast()` with no TypeId prefix,
//! no SignedGossipEnvelope wrapping, no BLAKE3 dedup. Transport fans
//! out to mesh peers without inspection. Every inbound copy reaches chat.
//!
//! **Traffic cost:** zero hash overhead at transport.
//! **Savings:** none — every inbound copy is delivered to chat.
//!
//! **Use for:** payload formats that are not SignedGossipEnvelope, or
//! when the caller has already handled dedup at the application layer,
//! or when the traffic pattern guarantees single-source delivery.
//!
//! **WARNING:** unmanaged broadcasts at high frequency WILL saturate the
//! chat event pipeline and IPC bus. If you are considering this method,
//! verify that your traffic pattern cannot use `broadcast_gossip_dedup`.
//! If unsure, use `broadcast_gossip_dedup` — the dedup overhead is
//! negligible compared to the cost of duplicate processing.

use std::time::Instant;

use rekindle_types::gossip_payload::{GossipPayload, SignedGossipEnvelope};
use super::{BroadcastReceipt, PlatformIO, SendReceipt, Confirm};
use crate::ChatError;

/// TypeId for deduped gossip broadcast frames. Transport recognizes this
/// byte and applies BLAKE3 content-hash dedup before delivery to chat.
const TYPEID_GOSSIP_DEDUP: u8 = 0x0A;

impl PlatformIO {
    /// Broadcast a gossip payload to all mesh peers with transport-level dedup.
    ///
    /// Constructs a SignedGossipEnvelope (signed with community pseudonym key),
    /// serializes with postcard, frames with TypeId 0x0A, broadcasts to mesh.
    ///
    /// Transport BLAKE3-hashes the framed bytes and suppresses duplicates
    /// arriving from multiple mesh peers. Chat's EventDedup provides a
    /// second layer of dedup on the semantic SubscriptionEvent level for
    /// events arriving via watch+gossip+poll convergence.
    ///
    /// Returns `Ok(BroadcastReceipt)` with delivery statistics. Partial
    /// delivery (some peers failed) is logged but returns Ok — the DHT
    /// write that preceded this call is the durability guarantee. Gossip
    /// is a real-time optimization, not a durability mechanism.
    pub async fn broadcast_gossip_dedup(
        &self,
        community: &str,
        payload: GossipPayload,
    ) -> Result<BroadcastReceipt, ChatError> {
        let start = Instant::now();
        let framed = self.build_gossip_frame(community, &payload, 3)?;

        let report = self.transport()
            .broadcast(community, &framed)
            .await
            .map_err(ChatError::Transport)?;

        let receipt = BroadcastReceipt {
            peers_sent: report.peers_sent,
            peers_failed: report.peers_failed,
            elapsed: start.elapsed(),
        };

        if receipt.peers_failed > 0 {
            tracing::debug!(
                community = &community[..12.min(community.len())],
                sent = receipt.peers_sent,
                failed = receipt.peers_failed,
                elapsed_ms = receipt.elapsed.as_millis(),
                "gossip_dedup: partial delivery — failed peers will discover via watch/poll"
            );
        }

        Ok(receipt)
    }

    /// Broadcast raw bytes to mesh peers without transport-level dedup.
    ///
    /// Transport fans out to mesh peers without inspection. Every inbound
    /// copy reaches chat. No SignedGossipEnvelope. No TypeId prefix.
    /// No BLAKE3 dedup.
    ///
    /// The caller is responsible for:
    /// - Signing the payload (if authentication is needed)
    /// - Framing (if TypeId routing is needed at chat's EventRouter)
    /// - Dedup (if the same bytes may arrive from multiple sources)
    ///
    /// If none of these are handled, every mesh peer receives unverified,
    /// unframed, undeduplicated bytes. This is rarely what you want.
    pub async fn broadcast_gossip_unmanaged(
        &self,
        community: &str,
        data: &[u8],
    ) -> Result<BroadcastReceipt, ChatError> {
        let start = Instant::now();

        let report = self.transport()
            .broadcast(community, data)
            .await
            .map_err(ChatError::Transport)?;

        let receipt = BroadcastReceipt {
            peers_sent: report.peers_sent,
            peers_failed: report.peers_failed,
            elapsed: start.elapsed(),
        };

        if receipt.peers_failed > 0 {
            tracing::debug!(
                community = &community[..12.min(community.len())],
                sent = receipt.peers_sent,
                failed = receipt.peers_failed,
                elapsed_ms = receipt.elapsed.as_millis(),
                "gossip_unmanaged: partial delivery"
            );
        }

        Ok(receipt)
    }

    /// Send a gossip payload directly to one peer (point-to-point).
    ///
    /// Same envelope construction as `broadcast_gossip_dedup` but ttl=0
    /// (no mesh forwarding) and sent via `transport.send_to_peer` instead
    /// of `transport.broadcast`.
    ///
    /// Used for: JoinAccepted (to joiner), JoinRejected (to joiner),
    /// MekTransfer (to requester), AdminKeypairGrant (to grantee).
    /// Any notification targeted at a specific known peer where mesh
    /// broadcast would be wasteful or leak information to uninvolved peers.
    pub async fn send_gossip_direct(
        &self,
        community: &str,
        target_peer_key: &str,
        payload: GossipPayload,
    ) -> Result<SendReceipt, ChatError> {
        let start = Instant::now();
        let framed = self.build_gossip_frame(community, &payload, 0)?;

        self.transport()
            .send_to_peer(target_peer_key, &framed)
            .await
            .map_err(ChatError::Transport)?;

        Ok(SendReceipt {
            peer_key: target_peer_key.to_string(),
            confirmed: Confirm::Accepted,
            elapsed: start.elapsed(),
        })
    }

    /// Build a framed gossip message ready for transport dispatch.
    ///
    /// The signature covers `payload_bytes` only. The `lamport_ts` field
    /// is unsigned metadata used for causal ordering — it is NOT a
    /// wall-clock timestamp and is NOT part of the authenticated data.
    ///
    /// Replay protection for gossip is provided by:
    /// 1. Transport-level BLAKE3 content dedup (same raw bytes → suppress)
    /// 2. Chat-level EventDedup (same semantic event → suppress)
    ///
    /// `lamport_ts` is set to 0 here. Transport may increment it from
    /// the community's gossip mesh Lamport clock before sending — this
    /// does not invalidate the signature because `lamport_ts` is not signed.
    fn build_gossip_frame(
        &self,
        community: &str,
        payload: &GossipPayload,
        ttl: u8,
    ) -> Result<Vec<u8>, ChatError> {
        let (pseudonym_hex, pseudonym_seed) = self.with_signing_key(|sk| {
            let seed = sk.pseudonym_seed(community);
            let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&seed)
                .map_err(|e| ChatError::Internal(format!("pseudonym keypair: {e}")))?;
            let hex = hex::encode(rekindle_ratchet::crypto::sign::public_key_bytes(&kp));
            Ok((hex, seed))
        })?;

        let payload_bytes = postcard::to_stdvec(&payload)
            .map_err(|e| ChatError::Serialization(format!("gossip payload: {e}")))?;

        let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&pseudonym_seed)
            .map_err(|e| ChatError::Internal(format!("sign keypair: {e}")))?;
        let sig = rekindle_ratchet::crypto::sign::sign_ec_prekey(&kp, &payload_bytes);

        let envelope = SignedGossipEnvelope {
            community_id: community.to_string(),
            sender_pseudonym: pseudonym_hex,
            payload_bytes,
            signature: sig.to_vec(),
            ttl,
            lamport_ts: 0,
        };

        let envelope_bytes = postcard::to_stdvec(&envelope)
            .map_err(|e| ChatError::Serialization(format!("gossip envelope: {e}")))?;

        let mut framed = Vec::with_capacity(1 + envelope_bytes.len());
        framed.push(TYPEID_GOSSIP_DEDUP);
        framed.extend_from_slice(&envelope_bytes);

        Ok(framed)
    }
}
