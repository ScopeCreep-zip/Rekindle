//! `GossipDeps` for the daemon — the second implementation of the
//! shared gossip stack.
//!
//! ## Why this exists
//!
//! The two tracks were gossiping in formats neither could read:
//!
//! | | desktop | daemon (before) |
//! |---|---|---|
//! | payload type | `CommunityEnvelope` | `GossipPayload` |
//! | payload encoding | Cap'n Proto | postcard |
//! | outer wrapper | `SignedEnvelope` | `SignedGossipEnvelope` |
//! | framing | unframed `app_message` | `TypeId::GossipBroadcast` |
//!
//! A desktop broadcast arrived at a daemon unframed and was rejected
//! before it could be parsed; a daemon broadcast arrived at a desktop
//! as a postcard blob `decode_signed_envelope` could not read. PATH 2
//! of the three-path model — the one the architecture calls the
//! "instant messaging feel" — worked in neither direction between
//! tracks. Same defect class as the cross-track MEK break: one track
//! migrated, the other left behind.
//!
//! Converging on the desktop's format rather than the daemon's is not
//! a coin flip. Cap'n Proto `CommunityEnvelope` is what the schemas in
//! `schemas/` describe, what `rekindle-gossip` (Tier 5) already
//! implements generically, and the only one of the two that can carry
//! `Control(..)` and `WatchRelay` — so it is the format the rest of the
//! architecture is written against.
//!
//! ## Shape
//!
//! Same as the daemon's other adapters: a `DaemonContext` handle and
//! delegation-only trait impl. Every method is backed by state the
//! daemon already had — `broadcast_mgr.meshes()` for the overlay,
//! `session` for pseudonyms, `signing_key` for the identity secret —
//! which is why this is an adapter rather than a port.

mod deps_impl;
mod state_mutations;
mod state_reads;

use std::sync::Arc;

use rekindle_gossip::resolve_gate::ResolveGate;
use rekindle_transport::TransportNode;

use super::dispatch::DaemonContext;

/// Gossip adapter over the live daemon context.
pub struct DaemonGossipAdapter {
    ctx: Arc<DaemonContext>,
    /// Single-flight route re-resolution.
    ///
    /// The trait requires one gate "per node for its lifetime" —
    /// coalescing concurrent re-resolutions only works if the racing
    /// callers share it. That holds because the worker in
    /// `daemon::gossip` builds **one** adapter and reuses it for every
    /// broadcast, rather than one per send.
    resolve_gate: ResolveGate,
}

impl DaemonGossipAdapter {
    #[must_use]
    pub fn new(ctx: Arc<DaemonContext>) -> Self {
        Self {
            ctx,
            resolve_gate: ResolveGate::new(),
        }
    }

    fn transport(&self) -> Option<Arc<TransportNode>> {
        self.ctx.transport.read().clone()
    }
}
