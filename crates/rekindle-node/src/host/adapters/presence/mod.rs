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
//!
//! The read side is the beat view ([`view`], plan E3.4 S3): the members'
//! beats the ring and hints deliver, expired locally by the 15-minute rule
//! in a task of the same scope, and the members a frontend shows outside
//! the ring, inspected once per beat period in another (plan E3.4 Q1,
//! `governance::on_screen`).

pub(crate) mod card_game;
pub(crate) mod mesh;
pub(crate) mod view;

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_codec::member_record::BeatState;
use rekindle_presence::community::{BeatEngine, BeatSink};
use rekindle_presence::SessionPresence;
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
        // A community our own pseudonym departed gets no beat (ADR 0011:
        // the leaver's last writes are its op and frontier, never a beat).
        self.host()
            .map(|h| {
                let mut targets = h.sync_targets();
                targets.retain(|c| !h.own_departed(c));
                targets
            })
            .unwrap_or_default()
    }

    fn status(&self) -> SessionStatus {
        self.ctx
            .upgrade()
            .map_or(SessionStatus::Offline, |ctx| ctx.own_presence.status())
    }

    async fn write_beat(&self, community: &str, state: BeatState) -> Result<(), String> {
        let host = self.host().ok_or("the host is gone")?;
        host.write_own_beat(community, state).await.map(drop)
    }

    async fn write_quit_beat(&self, community: &str) -> Result<(), String> {
        let host = self.host().ok_or("the host is gone")?;
        host.write_quit_beat(community).await
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
    })?;
    let expiry = ctx.this.clone();
    scope.spawn_with_token("presence expiry", move |stop| {
        view::run_expiry(expiry, stop)
    })?;
    let on_screen = ctx.this.clone();
    scope.spawn_with_token("presence on screen", move |stop| {
        run_on_screen(on_screen, stop)
    })?;
    let card_game = ctx.this.clone();
    scope.spawn_with_token("card game", move |stop| card_game::run(card_game, stop))
}

/// Inspect the members on screen outside the ring once per beat period
/// until `stop` (the unlock's scope). A member new on screen is read at
/// once by its report (`HostGovernance::set_members_on_screen`).
async fn run_on_screen(ctx: Weak<DaemonContext>, stop: tokio_util::sync::CancellationToken) {
    let period =
        std::time::Duration::from_millis(rekindle_presence::community::ON_SCREEN_INSPECT_MS);
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            () = tokio::time::sleep(period) => {}
        }
        let Some(ctx) = ctx.upgrade() else {
            return;
        };
        HostGovernance::new(ctx).inspect_on_screen().await;
    }
}

/// The shell's session signals (plan E3.4 S8-pre); `None` on `rekindled`,
/// which holds no voice session and shares nothing: its beats carry the
/// status alone.
pub(crate) fn session_presence(ctx: &DaemonContext) -> Option<Arc<dyn SessionPresence>> {
    ctx.shell.as_ref().map(|shell| shell.session_presence())
}

/// We joined or left a voice channel in `community` (plan E3.4 "Constant
/// rate": an explicit action writes at once): our beat says it now, and
/// the voice roster is handed what the members' beats show, so the members
/// already in the channel are found without waiting on their next beat.
pub(crate) async fn voice_changed(ctx: &DaemonContext, community: &str) {
    let engine = ctx.own_presence.beats.read().clone();
    if let Some(engine) = engine {
        if let Err(error) = engine.voice_changed(community).await {
            tracing::warn!(%error, "voice beat not written");
        }
    }
    view::reconcile_voice(ctx, community);
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
    } else {
        tracing::info!("quit beats skipped: no beat engine ran this unlock");
    }
}
