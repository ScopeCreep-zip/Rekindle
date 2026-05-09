//! Inbound handler trait implemented by the application layer.
//!
//! The transport crate handles all framing, authentication, dedup, and
//! decryption. By the time any method on [`InboundHandler`] is called,
//! the message has been:
//!
//! 1. Frame-decoded (version + type validated)
//! 2. Signature-verified (Ed25519, no exceptions)
//! 3. Dedup-checked (gossip only)
//! 4. Decrypted (if applicable: Signal for DM, MEK for channel/voice)
//!
//! The handler receives only authenticated, deserialized, plaintext payloads.
//!
//! # Async trait design
//!
//! This trait uses native `async fn` in trait (stable since Rust 1.75) with
//! explicit `+ Send` return bounds. This makes the trait non-object-safe
//! (`dyn InboundHandler` is not possible) but allows the dispatch loop to
//! spawn handler calls on the tokio runtime. The application layer must use
//! generics (`H: InboundHandler`), which is correct — there is exactly one
//! handler implementation per application.

use std::future::Future;

use crate::payload::dm::DmPayload;
use crate::payload::gossip::{GossipPayload, SignedGossipEnvelope};
use crate::payload::rpc::{CallResponse, InboundCall};
use crate::payload::voice::VoicePayload;

/// Identity of a verified inbound message sender.
#[derive(Debug, Clone)]
pub struct VerifiedSender {
    /// Ed25519 public key of the sender (hex-encoded, 64 chars).
    pub public_key: String,
    /// Display name if known from prior context (may be empty).
    pub display_name: String,
}

/// Notification of a transport-level event (not a user message).
#[derive(Debug, Clone)]
pub enum TransportEvent {
    /// Network attachment state changed.
    AttachmentChanged {
        state: String,
        is_attached: bool,
        public_internet_ready: bool,
    },
    /// One or more of our allocated private routes died.
    LocalRoutesDied { count: usize },
    /// One or more imported remote peer routes died.
    RemoteRoutesDied { peer_keys: Vec<String> },
    /// A DHT watch expired or was cancelled by the watching node.
    WatchDied { record_key: String },
}

/// Application-layer handler for all inbound transport events.
///
/// Implement this trait in the application crate to receive authenticated,
/// decrypted payloads from the transport layer. The transport guarantees
/// that every call to these methods has passed full authentication.
pub trait InboundHandler: Send + Sync + 'static {
    /// An authenticated DM payload arrived from a verified peer.
    /// DM message content arrives via DhtLog (Signal-encrypted), not here.
    /// This handler receives only ephemeral signals: typing, acks, unfriend.
    fn on_dm(&self, sender: &VerifiedSender, payload: DmPayload)
        -> impl Future<Output = ()> + Send;

    /// An authenticated gossip broadcast arrived from a community member.
    fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: GossipPayload,
        lamport_ts: u64,
    ) -> impl Future<Output = ()> + Send;

    /// A verified gossip envelope should be forwarded to the local mesh peers.
    ///
    /// The transport layer calls this BEFORE `on_gossip` so the message
    /// propagates even if handler processing is slow. The application layer
    /// should use its `Sender::broadcast_gossip` with its current peer set.
    /// The envelope's TTL has already been decremented by the transport.
    fn on_gossip_forward(&self, envelope: &SignedGossipEnvelope)
        -> impl Future<Output = ()> + Send;

    /// An authenticated, encrypted voice packet arrived.
    fn on_voice(&self, sender_key: &str, packet: VoicePayload)
        -> impl Future<Output = ()> + Send;

    /// An authenticated RPC request arrived, expecting a response.
    fn on_call(
        &self,
        sender_pseudonym: Option<&str>,
        request: InboundCall,
    ) -> impl Future<Output = CallResponse> + Send;

    /// A DHT record subkey changed (watch notification or inspect diff).
    fn on_value_change(
        &self,
        record_key: &str,
        changed_subkeys: Vec<u32>,
        first_value: Option<Vec<u8>>,
    ) -> impl Future<Output = ()> + Send;

    /// A transport-level event occurred (network state, route death).
    fn on_event(&self, event: TransportEvent)
        -> impl Future<Output = ()> + Send;
}
