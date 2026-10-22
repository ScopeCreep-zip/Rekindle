//! The Host's governance adapter (plan E3.1 B1): one implementation of
//! `rekindle_governance_runtime::GovernanceRuntimeDeps` over the
//! [`DaemonContext`] — its community store (`host::community::store`),
//! database, vault, transport and journal — used by both hosts. It replaces
//! the daemon's `daemon/governance_adapter/` and the desktop's (B1/B2; the
//! desktop's copy deleted at B2f), and the login stages that drive hydration
//! over it are C1 (`loader`, the warm cache) and C2a (`login_stage`, the
//! network part; `catchup` the history catch-up it schedules).
//!
//! File split: this module holds the struct, constructor and shared
//! helpers; `state_reads` / `state_mutations` / `dht` / `roles` / `events`
//! hold the bodies; `lifecycle` holds the gossip, permission, join and
//! hydration bodies (its `governance_entries_cache` save is what C1's warm
//! restore reads back); `registry` the discovered members and the stored registry keypair
//! (B2d); `membership` / `membership_route` the membership control
//! messages and `state_builder` a created community's state (B2e); `mek` the key installs, which move to the MEK
//! adapters at E3.5; `community_sync` the sync loop's community phase, `invite_expiry` the
//! invite expiry timer and `mek_recovery` the key catch-up (C2b); `game_servers` a community's game servers (H1); `deps_impl` is the single trait impl and delegates; `create_leave` and `views` the
//! community window's create, leave and reads (E3.1 D1b); `audit` / `audit_rows` the audit log
//! and the notification default (F4).

use std::sync::Arc;

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_transport::TransportNode;

use crate::daemon::dispatch::DaemonContext;

pub mod audit;
mod audit_rows;
pub(crate) mod community_pictures;
pub(crate) mod community_profile;
pub(crate) mod community_sync;
pub mod deps_impl;
mod dht;
pub use dht::recent_channel_messages;
mod catchup;
mod community_events;
mod community_info;
mod create_leave;
mod event_control;
mod event_reminders;
mod event_rsvps;
pub(crate) use community_info::InfoEdit;
pub(crate) mod channel_admin;
mod events;
pub(crate) mod game_servers;
mod invite_expiry;
mod invites;
mod join;
mod lifecycle;
pub(crate) mod loader;
pub(crate) mod login_stage;
mod mek;
mod mek_recovery;
mod membership;
mod membership_roster;
mod membership_route;
pub(crate) mod onboarding;
pub(crate) mod onboarding_map;
pub mod projection;
mod registry;

pub mod moderation;
pub mod role_ops;
mod roles;
mod state_builder;
mod state_mutations;
mod state_reads;
mod views;
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
