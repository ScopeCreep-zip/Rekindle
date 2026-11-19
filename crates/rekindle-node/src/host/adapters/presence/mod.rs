//! Community presence in the Host (plan E3.4 "Community presence moves into
//! the Host"), on both modes (ADR 0010): the beat engine
//! (`rekindle_presence::community::beat`) over the Host's record port. One
//! engine per unlock, started beside the member routes writer
//! (`governance::community_sync::start`) and stopped with the unlock's
//! scope. Its sink writes our `M`/1 in every community we hold a member
//! record in ([`HostGovernance::write_own_beat`]).
//!
//! The engine is installed in [`OwnPresence`](crate::host::own_presence)'s
//! slot so a picked status reaches it at once, a leave writes its
//! `Departed` beat, and the teardown writes the quit's `Offline` beat.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_codec::member_record::BeatState;
use rekindle_presence::community::{BeatEngine, BeatSink};
use rekindle_types::presence::SessionStatus;

use super::governance::HostGovernance;
use crate::daemon::dispatch::DaemonContext;

/// The unlock's beat engine, while one runs or until the teardown's quit.
pub(crate) type BeatSlot = Arc<RwLock<Option<Arc<BeatEngine>>>>;

/// The engine's sink. It holds the context weakly: the engine sits in the
/// context's own presence slot.
struct HostBeats {
    ctx: Weak<DaemonContext>,
}

impl HostBeats {
    fn host(&self) -> Option<HostGovernance> {
        self.ctx.upgrade().map(HostGovernance::new)
    }
}

#[async_trait]
impl BeatSink for HostBeats {
    fn communities(&self) -> Vec<String> {
        self.host().map(|h| h.sync_targets()).unwrap_or_default()
    }

    fn status(&self) -> SessionStatus {
        self.ctx
            .upgrade()
            .map_or(SessionStatus::Offline, |ctx| ctx.own_presence.status())
    }

    async fn write_beat(&self, community: &str, state: BeatState) -> Result<(), String> {
        let host = self.host().ok_or("the host is gone")?;
        host.write_own_beat(community, state).await
    }
}

/// Start the unlock's beat engine in `scope` and install it.
///
/// # Errors
/// The scope is closed (the unlock is ending).
pub(crate) fn start(
    ctx: &DaemonContext,
    scope: &Arc<rekindle_lifecycle::SessionScope>,
) -> Result<(), rekindle_lifecycle::ScopeClosed> {
    let sink = Arc::new(HostBeats {
        ctx: ctx.this.clone(),
    });
    let engine = Arc::new(BeatEngine::new(sink, Arc::clone(&ctx.rng)));
    *ctx.own_presence.beats.write() = Some(Arc::clone(&engine));
    scope.spawn_with_token("presence beats", move |stop| async move {
        engine.run(stop).await;
    })
}

/// Write our `Departed` beat in `community`, which we are leaving.
pub(crate) async fn depart(ctx: &DaemonContext, community: &str) {
    let engine = ctx.own_presence.beats.read().clone();
    if let Some(engine) = engine {
        if let Err(error) = engine.depart(community).await {
            tracing::warn!(%error, "departed beat not written");
        }
    }
}

/// The quit's `Offline` beat in every community (unless a hiding status
/// already wrote it), once the engine stopped; the slot is emptied.
pub(crate) async fn quit(slot: &BeatSlot) {
    let engine = slot.write().take();
    if let Some(engine) = engine {
        engine.quit().await;
    }
}
