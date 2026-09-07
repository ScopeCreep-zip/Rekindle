//! Daemon-side governance runtime adapter.
//!
//! Implements `rekindle_governance_runtime::GovernanceRuntimeDeps`
//! against the live `DaemonContext`, so the daemon track runs the same
//! v2.0 flat-SMPL governance the Tauri host does. Before this existed,
//! `rekindle-node` had no dependency on `rekindle-governance-runtime`
//! at all and drove membership through the v1.0 coordinator accessors
//! (`read/write_member_index`, `read/write_mek_vault`).
//!
//! **This is a second adapter behind one trait, not a duplicate.**
//! `docs/architecture/services-pattern.md` §2 defines an adapter as
//! implementing a `Deps` trait "against the live `AppState` +
//! `AppHandle` + `DbPool`" — the **Schwarzschild boundary**. The shared
//! contract between the two tracks is the *trait*; each host keeps its
//! own state behind it, which is why the trait exchanges snapshots
//! (`MekSnapshot`, `OnlineMemberSnapshot`, an all-`Option`
//! `CommunityMembership`) and opaque `String`/`Vec<u8>` for every Veilid
//! type rather than sharing structs. Hoisting the Tauri host's
//! `CommunityState` into a shared crate would drag SQLite-backed,
//! `AppHandle`-shaped state across that horizon.
//!
//! Where the daemon genuinely differs from the Tauri host:
//!
//! | Concern | Tauri host | Daemon |
//! |---|---|---|
//! | Persisted membership | `CommunityState` in SQLite | `Session.communities` in `session.json` |
//! | Runtime governance cache | `CommunityState.governance_state` | [`crate::daemon::community_runtime`] |
//! | Secrets at rest | Stronghold | `rekindle-vault` / OS keyring |
//! | UI events | `app.emit` → SolidJS | `event_router` → IPC subscribers |
//!
//! File split follows the documented adapter shape: this module holds
//! the struct, constructor and shared helpers; `state_reads` /
//! `state_mutations` / `dht` / `mek` hold the bodies; `deps_impl` is the
//! single trait impl and delegates to them.

use std::sync::Arc;

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_transport::TransportNode;

use crate::daemon::dispatch::DaemonContext;

pub mod deps_impl;
mod dht;
mod events;
mod lifecycle;
mod mek;
mod roles;
mod state_mutations;
mod state_reads;

/// Subkeys watched per community record.
///
/// Covers a full SMPL segment: under `o_cnt: 0` every subkey `0..254` is
/// a member slot and which member writes next is not knowable in
/// advance, so the watch spans the whole width rather than a guessed
/// subset.
pub(super) const SLOT_WATCH_WIDTH: u32 = 255;

/// Adapter holding everything the trait methods need. One field, because
/// `DaemonContext` already aggregates the transport, session, MEK cache,
/// signing key and community runtime state.
///
/// Borrows rather than owning an `Arc`: dispatch runs as
/// `dispatch(ctx: &DaemonContext, …)` and the daemon's single `Arc` never
/// reaches a handler, so an owning adapter would mean threading `Arc`s
/// through the whole request chain. This is a short-lived view over the
/// context for one operation, which is what the lifetime says.
pub struct DaemonGovernanceAdapter<'a> {
    pub(super) ctx: &'a DaemonContext,
}

impl<'a> DaemonGovernanceAdapter<'a> {
    pub fn new(ctx: &'a DaemonContext) -> Self {
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
