//! The Host's governance adapter (plan E3.1 B1): one implementation of
//! `rekindle_governance_runtime::GovernanceRuntimeDeps` over the
//! [`DaemonContext`] — its community store (`host::community::store`),
//! database, vault, transport and journal — used by both hosts. It replaces
//! the daemon's `daemon/governance_adapter/`; the desktop's adapter merges
//! into it (B1/B2), and the login stage that drives hydration over it is C1.
//!
//! File split: this module holds the struct, constructor and shared
//! helpers; `state_reads` / `state_mutations` / `dht` / `roles` / `events`
//! hold the bodies; `lifecycle` holds the gossip, permission, join and
//! hydration bodies (its `governance_entries_cache` save is what C1's warm
//! restore reads back); `registry` the discovered members and the stored registry keypair
//! (B2d); `mek` the key installs, which move to the MEK
//! adapters at E3.5; `deps_impl` is the single trait impl and delegates.

use std::sync::Arc;

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_transport::TransportNode;

use crate::daemon::dispatch::DaemonContext;

pub mod deps_impl;
mod dht;
pub use dht::recent_channel_messages;
mod events;
mod lifecycle;
pub(crate) mod loader;
mod mek;
pub mod projection;
mod registry;

mod roles;
mod state_mutations;
mod state_reads;
mod watch;

/// The Host, as the governance runtime sees it.
#[derive(Clone)]
pub struct HostGovernance {
    pub(super) ctx: Arc<DaemonContext>,
}

impl HostGovernance {
    /// The Host over `ctx`.
    pub(crate) fn new(ctx: Arc<DaemonContext>) -> Self {
        Self { ctx }
    }

    /// The transport node, or a typed error when Veilid has not started.
    ///
    /// Every DHT-facing trait method needs this, and the failure is
    /// always the same: the daemon accepted a request before/after the
    /// node was up. `Adapter` rather than a bespoke variant so the
    /// message reaches the IPC client verbatim.
    pub(super) fn transport(&self) -> Result<Arc<TransportNode>, GovernanceRuntimeError> {
        self.ctx
            .transport
            .read()
            .clone()
            .ok_or_else(|| GovernanceRuntimeError::Adapter("transport not started".into()))
    }
}

impl crate::host::Host {
    /// The governance runtime's deps over this Host.
    #[must_use]
    pub fn governance(&self) -> HostGovernance {
        HostGovernance::new(Arc::clone(&self.ctx))
    }
}
