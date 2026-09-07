//! Departure-triggered MEK rotation on the daemon track.
//!
//! `communities-channels.md` makes this the forward-secrecy mechanism:
//! when a member departs, every remaining member independently computes
//! `blake3(departed || own_pseudonym)`, the lowest hash rotates, and the
//! new key is delivered to each peer by `app_call`. The shared
//! implementation is `rekindle-mek-rotation`; this module is the
//! daemon's adapter for it.
//!
//! Before this, the daemon had none. `spawn_text_mek_rotation_for_ban`
//! traced a line and returned, and the leave RPC rekeyed by writing a
//! v1.0 registry MEK vault that only an "operator" could write. A
//! banned member on the daemon track kept a working key.
//!
//! ## Why a worker channel rather than a spawned task
//!
//! Rotation is fire-and-forget from the caller's perspective — a ban has
//! already landed and must not be undone by a rotation failure — but it
//! is *slow*: `wait_for_rotation_slot` sleeps through the cascade levels
//! so a lower-ranked member only rotates if the elected one stays quiet.
//! Running that inline would hold an IPC handler open for tens of
//! seconds.
//!
//! Detaching it needs `'static` state, and the daemon's single
//! `Arc<DaemonContext>` deliberately stops at `dispatch`: 68 handler
//! functions take `&DaemonContext`, and threading an `Arc` through all
//! of them to reach two call sites would be a poor trade. So the
//! triggers send a request on a channel, and one worker — spawned at
//! startup where the `Arc` does exist — owns it and does the waiting.
//! Both current triggers (moderation ban, leave notification) and any
//! future one only need a `Sender`.

mod cache;
mod deps_impl;

use std::sync::Arc;

use crate::daemon::dispatch::DaemonContext;

pub use cache::{DaemonMekPersist, MekCacheAdapter};

/// A departure that invalidates the current MEK.
#[derive(Debug, Clone)]
pub struct MekRotationRequest {
    /// Governance key of the affected community.
    pub community_id: String,
    /// The member who left or was banned, hex-encoded pseudonym.
    pub departed_pseudonym_hex: String,
}

/// Handle used by triggers to ask for a rotation.
pub type MekRotationSender = tokio::sync::mpsc::UnboundedSender<MekRotationRequest>;

/// Receiving half, owned by the worker.
pub type MekRotationReceiver = tokio::sync::mpsc::UnboundedReceiver<MekRotationRequest>;

/// Create the trigger/worker channel pair.
#[must_use]
pub fn channel() -> (MekRotationSender, MekRotationReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// The daemon's `MekDistributeDeps` implementation.
///
/// Owns an `Arc<DaemonContext>` rather than borrowing, because it
/// outlives the request that triggered it — see the module docs.
pub struct DaemonMekAdapter {
    pub(super) ctx: Arc<DaemonContext>,
    pub(super) cache: Arc<dyn rekindle_mek_rotation::ChannelMekCache>,
    pub(super) persist: Arc<dyn rekindle_mek_rotation::MekPersist>,
}

impl DaemonMekAdapter {
    #[must_use]
    pub fn new(ctx: Arc<DaemonContext>) -> Self {
        let cache: Arc<dyn rekindle_mek_rotation::ChannelMekCache> =
            Arc::new(MekCacheAdapter::new(Arc::clone(&ctx.mek_cache)));
        let persist: Arc<dyn rekindle_mek_rotation::MekPersist> = Arc::new(DaemonMekPersist);
        Self {
            ctx,
            cache,
            persist,
        }
    }
}

/// Drain rotation requests, one at a time, until the daemon shuts down.
///
/// Serialised deliberately. Two concurrent rotations of the same
/// community would race to bump the generation and each would see the
/// other's `MEKGenerationBump` as a competing write; running them in
/// order means the second observes the first's generation and yields
/// through `wait_for_rotation_slot` instead of duplicating the work.
pub async fn run_worker(ctx: Arc<DaemonContext>, mut rx: MekRotationReceiver) {
    tracing::info!("MEK rotation worker started");
    while let Some(request) = rx.recv().await {
        let adapter = DaemonMekAdapter::new(Arc::clone(&ctx));
        let short = &request.community_id[..16.min(request.community_id.len())];
        match rekindle_mek_rotation::rotate_text_mek_for_departure(
            &adapter,
            &request.community_id,
            &request.departed_pseudonym_hex,
        )
        .await
        {
            Ok(()) => tracing::debug!(community = %short, "departure rotation finished"),
            // Not an error path in the common case: losing the cascade
            // to a better-ranked peer, or having no online recipients,
            // both end here and both are correct outcomes.
            Err(e) => tracing::debug!(
                community = %short,
                error = %e,
                "departure rotation did not complete"
            ),
        }
    }
    tracing::info!("MEK rotation worker stopped");
}
