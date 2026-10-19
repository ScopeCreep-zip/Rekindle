//! Inbound handler trait implemented by the application layer.
//!
//! The transport crate handles all framing, authentication, dedup, and
//! decryption. By the time any method on [`InboundHandler`] is called,
//! the message has been:
//!
//! 1. Frame-decoded (version + type validated)
//! 2. Signature-verified (Ed25519, no exceptions)
//! 3. Dedup-checked (gossip only)
//! 4. Decrypted (if applicable: MEK for channel/voice)
//!
//! The handler receives only authenticated, deserialized, plaintext payloads,
//! except a 1:1 message (`on_direct`), which carries its own signed envelope
//! and is opened by `operations::direct` on both hosts (plan E2.1).
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

use crate::payload::rpc::{CallResponse, InboundCall};
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_codec::community::envelope::SignedEnvelope;

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
    /// One of our own routes changed state (allocated, died, failed): the
    /// network status a window shows changed though attachment did not.
    RoutesChanged,
    /// One or more of our allocated private routes died.
    LocalRoutesDied { count: usize },
    /// One or more imported remote peer routes died.
    RemoteRoutesDied { peer_keys: Vec<String> },
}

/// Application-layer handler for all inbound transport events.
///
/// Implement this trait in the application crate to receive authenticated,
/// decrypted payloads from the transport layer. The transport guarantees
/// that every call to these methods has passed full authentication.
pub trait InboundHandler: Send + Sync + 'static {
    /// Our identity key while an identity is unlocked. Signed framed RPCs
    /// are verified as addressed to it; with none, they are dropped.
    fn local_identity(&self) -> Option<[u8; 32]>;

    /// The host's wall clock (plan C9.2): the time buckets of the gossip
    /// dedup key.
    fn clock(&self) -> &dyn rekindle_utils::Clock;

    /// A 1:1 `app_message` arrived: `datagram` starts with the `'D'` tag
    /// (`operations::dm::DIRECT_TAG`). The handler opens it with
    /// `operations::direct::receive`, which verifies the envelope itself.
    fn on_direct(&self, datagram: &[u8]) -> impl Future<Output = ()> + Send;

    /// A 1:1 `app_call` arrived (tagged as for [`Self::on_direct`]): the
    /// reply, or `None` for a NAK.
    fn on_direct_call(&self, datagram: &[u8]) -> impl Future<Output = Option<Vec<u8>>> + Send;

    /// An authenticated gossip broadcast arrived from a community member.
    fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        envelope: CommunityEnvelope,
        lamport_ts: u64,
    ) -> impl Future<Output = ()> + Send;

    /// A verified gossip envelope should be forwarded to the local mesh peers.
    ///
    /// The transport layer calls this BEFORE `on_gossip` so the message
    /// propagates even if handler processing is slow. The application layer
    /// should use its `Sender::broadcast_gossip` with its current peer set.
    /// The envelope's TTL has already been decremented by the transport,
    /// and directed payloads (`CommunityEnvelope::is_directed`) never
    /// reach here — §10.6.
    ///
    /// Epidemic delivery depends on this hop. A sender fans out to
    /// `min(N, 6)` peers and relies on receivers re-broadcasting within
    /// the 5-hop TTL; without it a message reaches six people in a
    /// forty-member community.
    fn on_gossip_forward(&self, envelope: &SignedEnvelope) -> impl Future<Output = ()> + Send;

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
    fn on_event(&self, event: TransportEvent) -> impl Future<Output = ()> + Send;
}
