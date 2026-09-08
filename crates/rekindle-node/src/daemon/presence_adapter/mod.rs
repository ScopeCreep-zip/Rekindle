//! `CommunityPresenceDeps` for the daemon.
//!
//! ## Why this exists
//!
//! `rekindle-presence` is a Tier-5 crate holding the community presence
//! orchestrators — the registry scan, the W26 verification, the gossip
//! overlay rebuild, the roster diff. Until now only src-tauri
//! implemented its `CommunityPresenceDeps`, so **the daemon had no
//! presence subsystem at all**: no registry scan, no validated
//! membership view, no materialised roster.
//!
//! That is why the daemon still read the v1.0 member index. The index
//! was standing in for the whole subsystem, and no accessor swap could
//! retire it — there was nothing on this track to swap *to*.
//!
//! It is the same gap class as `GovernanceRuntimeDeps` before 2.4 and
//! `MekDistributeDeps` before the rotation work: a shared crate the
//! desktop drives and the daemon does not. Closing it is what the plan
//! means by one implementation, two hosts.
//!
//! ## Where a member actually comes from
//!
//! There is no membership ledger. `communities-overview.md` describes
//! the registry as *"Single struct overwritten per heartbeat. Never
//! grows"*, and under `AdmissionMode::Open` governance carries no entry
//! for a joiner. So membership is a **predicate over registry rows** —
//! a validly-signed `MemberPresence` whose author is neither banned nor
//! departed — evaluated once per poll tick by
//! `parse_and_classify_row`, with the answer left in
//! [`crate::daemon::community_runtime`].
//!
//! Everything downstream reads that materialised roster. Re-deriving it
//! per request would verify every signature again to answer a question
//! the poll already answered.
//!
//! ## What the daemon genuinely lacks
//!
//! Some of this trait describes stores the daemon does not have, and
//! those return empty rather than pretending:
//!
//! | Surface | Why empty |
//! |---|---|
//! | channel message catch-up | no local message store to catch up *into* — the DHT read itself works, both tracks now writing SMPL channel segment records |
//! | event RSVPs | no event store |
//! | voice roster | no voice engine — same gap as `MekDistributeDeps::voice_recipients` |
//!
//! Each is a recorded capability gap, not a stub to be forgotten: the
//! presence poll degrades to "roster and gossip overlay only", which is
//! exactly what the daemon needs to stop reading the member index.

mod deps_impl;
mod events;
mod gossip;
mod lifecycle;
mod registry;
mod state_reads;
pub mod supervisor;

use std::sync::Arc;

use crate::daemon::dispatch::DaemonContext;

/// Presence adapter over the daemon's context.
///
/// Owned rather than borrowed: the presence poll is a long-lived
/// spawned loop, so it needs `'static` state — the same reason
/// `DaemonMekAdapter` holds an `Arc`, and unlike the short-lived
/// `DaemonGovernanceAdapter` which lives for one request.
pub struct DaemonPresenceAdapter {
    pub(super) ctx: Arc<DaemonContext>,
}

impl DaemonPresenceAdapter {
    #[must_use]
    pub fn new(ctx: Arc<DaemonContext>) -> Self {
        Self { ctx }
    }

    /// The transport node, or `None` before Veilid starts.
    pub(super) fn transport(&self) -> Option<Arc<rekindle_transport::TransportNode>> {
        self.ctx.transport.read().as_ref().map(Arc::clone)
    }
}
