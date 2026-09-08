//! The daemon's outbound gossip — PATH 2 of the three-path model.
//!
//! One entry point replacing three partial ones. `presence_adapter`,
//! `governance_adapter` and `mek_rotation` each implemented
//! `send_to_mesh` by hand-translating the `CommunityEnvelope` variants
//! they happened to need into transport's postcard `GossipPayload`
//! helpers, each ending in an error for anything else ("only Control
//! envelopes are gossiped", "only MEKRotated is wired on the daemon
//! track"). Three translations of one operation, each with a different
//! hole in it.
//!
//! They now all send here, and this encodes the envelope the way the
//! desktop does — Cap'n Proto `CommunityEnvelope` inside a
//! `SignedEnvelope`, unframed on `app_message`. That is what makes
//! cross-track gossip work at all: before this the daemon put postcard
//! bytes inside a `TypeId::GossipBroadcast` frame, which no desktop peer
//! could parse, and rejected the desktop's unframed Cap'n Proto in
//! return. Every variant now crosses, including `WatchRelay` (§14.3) and
//! the whole `Control(..)` family.
//!
//! ## Why a worker channel
//!
//! `rekindle_gossip::send_to_mesh_raw` spawns its per-peer fan-out, so
//! the `Arc<D>` it holds must be `'static`. The daemon's single
//! `Arc<DaemonContext>` deliberately stops at `dispatch` — 68 handler
//! functions take `&DaemonContext` — and the three `Deps::send_to_mesh`
//! methods are sync, so none of them can await or produce an `Arc`. Same
//! constraint the MEK rotation hit, same resolution: senders push onto a
//! channel, and one worker spawned at startup owns the `Arc`.
//!
//! Reusing one adapter across every broadcast is also what makes
//! `GossipDeps::resolve_gate` do its job — the single-flight gate only
//! coalesces if the racing callers share it.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, SignedEnvelope};

use super::dispatch::DaemonContext;
use super::gossip_adapter::DaemonGossipAdapter;

/// One broadcast for the worker.
#[derive(Debug, Clone)]
pub enum GossipRequest {
    /// Something we are originating — sign it and fan it out.
    ///
    /// Boxed because `CommunityEnvelope` is far larger than a
    /// `SignedEnvelope` (it inlines every control variant, the largest
    /// being a bootstrap bundle), and an unboxed enum would pay that
    /// size on every forward too.
    Originate {
        community_id: String,
        envelope: Box<CommunityEnvelope>,
    },
    /// Something a peer signed that we are relaying one hop further.
    ///
    /// Kept distinct from `Originate` because a forward must **not** be
    /// re-signed: the envelope carries its author's signature, and
    /// replacing it would make every relayed message look like ours and
    /// destroy attribution. Only the TTL differs from what we received.
    Forward(SignedEnvelope),
}

/// Handle used by adapters to broadcast.
pub type GossipSender = tokio::sync::mpsc::UnboundedSender<GossipRequest>;

/// Receiving half, owned by the worker.
pub type GossipReceiver = tokio::sync::mpsc::UnboundedReceiver<GossipRequest>;

/// Create the sender/worker channel pair.
#[must_use]
pub fn channel() -> (GossipSender, GossipReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Queue an envelope for broadcast. Never blocks; never fails loudly.
///
/// Gossip is PATH 2 — "Durability: None" in the architecture — so a
/// dropped broadcast is a latency event, not a correctness one. The
/// SMPL write on PATH 1 already carries the content and PATH 3 picks it
/// up. Callers therefore get `()` rather than a `Result` they would only
/// log.
pub fn send(tx: &GossipSender, community_id: &str, envelope: &CommunityEnvelope) {
    if tx
        .send(GossipRequest::Originate {
            community_id: community_id.to_string(),
            envelope: Box::new(envelope.clone()),
        })
        .is_err()
    {
        tracing::debug!(
            community = %&community_id[..16.min(community_id.len())],
            "gossip: worker gone, dropping broadcast"
        );
    }
}

/// Relay a peer's envelope one hop further, unmodified but for the TTL
/// the transport already decremented.
pub fn forward(tx: &GossipSender, envelope: SignedEnvelope) {
    let _ = tx.send(GossipRequest::Forward(envelope));
}

/// Drain broadcast requests until the daemon shuts down.
///
/// One adapter for the worker's lifetime — see the module docs on
/// `resolve_gate`. Each request is awaited to the point where the
/// crate spawns its own fan-out, so a slow peer cannot stall the queue.
pub async fn run_worker(ctx: Arc<DaemonContext>, mut rx: GossipReceiver) {
    let adapter = Arc::new(DaemonGossipAdapter::new(ctx));
    tracing::info!("gossip worker started");
    while let Some(request) = rx.recv().await {
        match request {
            GossipRequest::Originate {
                community_id,
                envelope,
            } => {
                if let Err(error) =
                    rekindle_gossip::send_to_mesh(Arc::clone(&adapter), &community_id, &envelope)
                        .await
                {
                    tracing::warn!(
                        community = %&community_id[..16.min(community_id.len())],
                        %error,
                        "gossip: broadcast pipeline error"
                    );
                }
            }
            GossipRequest::Forward(signed) => {
                let community_id = signed.community_id.clone();
                rekindle_gossip::send_to_mesh_raw(Arc::clone(&adapter), &community_id, signed);
            }
        }
    }
    tracing::info!("gossip worker stopped");
}
