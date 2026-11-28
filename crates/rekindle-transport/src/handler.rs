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
//! Gossip and signed RPC reach the handler after verification by the
//! transport. Direct messages, media datagrams and bare media-key calls
//! carry the authentication that their receiving handler must verify.
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
use crate::subscriptions::media_key::MediaKeyReply;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, SignedEnvelope};

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
/// Each method documents the authentication performed before dispatch
/// and what remains for the application to check.
pub trait InboundHandler: Send + Sync + 'static {
    /// Our identity key while an identity is unlocked. Signed framed RPCs
    /// are verified as addressed to it; with none, they are dropped.
    fn local_identity(&self) -> Option<[u8; 32]>;

    /// The host's wall clock (plan C9.2): the time buckets of the gossip
    /// dedup key.
    fn clock(&self) -> &dyn rekindle_utils::Clock;

    /// The unlocked session's scope (a closed one while none is unlocked):
    /// a bare request's handler runs on it, so its work goes on to
    /// completion past [`crate::operations::direct::CALL_DEADLINE`] (plan
    /// JP1, review RV5 H1), as a 1:1 call's does on its host's scope.
    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope>;

    /// A 1:1 `app_message` arrived: `datagram` starts with the `'D'` tag
    /// (`operations::dm::DIRECT_TAG`). The handler opens it with
    /// `operations::direct::receive`, which opens the frame by its carrier.
    fn on_direct(&self, datagram: &[u8]) -> impl Future<Output = ()> + Send;

    /// A 1:1 `app_call` arrived (tagged as for [`Self::on_direct`]) over
    /// our route `route` (Veilid's route id, `None` with no private route):
    /// the reply (`operations::direct`'s receipt, `NAK` or `RETRY_LATER`,
    /// or a frame), or `None` for no reply at all (no host to judge it).
    fn on_direct_call(
        &self,
        datagram: &[u8],
        route: Option<&str>,
    ) -> impl Future<Output = Option<Vec<u8>>> + Send;

    /// A media datagram arrived (plan E4.1 S3): `raw` starts with a media
    /// tag (`rekindle_codec::ingress::Ingress::Media`), tag included, and
    /// `arrived` is when the dispatch loop took it, before any queue adds a
    /// delay the bandwidth estimator would measure. Synchronous on purpose:
    /// the handler verifies and `try_send`s onto its media owner and
    /// returns, so the serial dispatch loop never waits on media, and the
    /// app-call lane keeps media from ever waiting on a call.
    fn on_media(&self, raw: &[u8], arrived: std::time::Instant);

    /// An admitted bare media key or request (ADR 0016). The voice
    /// session checks the key signature or requester's roster entry.
    fn on_media_key_call(
        &self,
        payload: &ControlPayload,
    ) -> impl Future<Output = MediaKeyReply> + Send;

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

    /// A ring watcher's `HeadAdvanced` hint (plan E3.2 S7, `headAdvanced
    /// @76`) arrived, its envelope verified: `true` when the hint passed
    /// the member gate, the sender's gossip rate limit and its own
    /// signature, so it is relayed one hop further. It is gated before it
    /// is relayed, never relayed first (register S7l): a non-member's or a
    /// flooder's hint stops at the first member.
    fn on_head_advanced(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        hint: &[u8],
    ) -> impl Future<Output = bool> + Send;

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
