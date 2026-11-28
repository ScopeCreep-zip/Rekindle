//! The game stage's switch (plan E7.2): detection runs only while the
//! identity's `game_detection_enabled` is on. Off, no scanner task exists
//! and the process table is never read; `SetGameDetection` starts or stops
//! it at once, with no restart. The stage runs in its own child of the
//! unlock scope, so a lock ends it with everything else.

use std::sync::Arc;

use rekindle_lifecycle::SessionScope;

use super::game::{Publisher, SessionLog};
use crate::daemon::dispatch::DaemonContext;

/// The running stage, if detection is on.
#[derive(Default)]
pub(crate) struct GameControl {
    running: parking_lot::Mutex<Option<Running>>,
}

struct Running {
    scope: Arc<SessionScope>,
    publisher: Arc<Publisher>,
}

impl GameControl {
    /// Whether the scanner task is running (the tests' probe).
    #[cfg(test)]
    pub(crate) fn is_scanning(&self) -> bool {
        self.running
            .lock()
            .as_ref()
            .is_some_and(|running| !running.scope.is_closed() && !running.scope.is_empty())
    }

    /// Start the scanner in a child of `unlock` for `owner`, unless it runs.
    ///
    /// # Errors
    /// The unlock scope is closed (the identity is locking).
    pub(crate) fn start(
        &self,
        ctx: &DaemonContext,
        unlock: &Arc<SessionScope>,
        owner: String,
    ) -> Result<(), String> {
        let mut running = self.running.lock();
        if running
            .as_ref()
            .is_some_and(|running| !running.scope.is_closed())
        {
            return Ok(());
        }
        let sessions = SessionLog::new(
            Arc::clone(&ctx.db),
            owner,
            Arc::clone(&ctx.clock),
            Arc::new(super::adapters::friends::HostFriends::of(ctx)),
        );
        let publisher = Arc::new(Publisher::new(ctx.own_presence.clone(), sessions));
        let scope = unlock.child("game detection");
        let task = Arc::clone(&publisher);
        scope
            .spawn_with_token("game detection", |stop| super::game::run(task, stop))
            .map_err(|e| e.to_string())?;
        *running = Some(Running { scope, publisher });
        Ok(())
    }

    /// Stop the scanner: its task ends (recording the open session), then
    /// the game it showed is cleared.
    pub(crate) async fn stop(&self) {
        let Some(running) = self.running.lock().take() else {
            return;
        };
        running.scope.close_and_wait().await;
        running.publisher.withdraw().await;
    }

    /// Sharing or the hide list changed: the detected game is shown or
    /// hidden as they now allow.
    pub(crate) async fn reapply(&self) {
        let publisher = self
            .running
            .lock()
            .as_ref()
            .map(|running| Arc::clone(&running.publisher));
        if let Some(publisher) = publisher {
            publisher.reapply().await;
        }
    }

    /// The identity locked: forget the stage (its scope ended with the
    /// unlock scope).
    pub(crate) fn forget(&self) {
        self.running.lock().take();
    }
}

/// Start the stage at unlock if `owner`'s detection is on; whether it
/// started. Unreadable preferences start nothing (no scan without a
/// readable yes).
///
/// # Errors
/// The unlock scope is closed.
pub(crate) async fn start_if_enabled(
    ctx: &DaemonContext,
    unlock: &Arc<SessionScope>,
    owner: String,
) -> Result<bool, String> {
    let db = ctx.db.current().map_err(|e| e.to_string())?;
    let read = owner.clone();
    let enabled = db
        .call(move |conn| rekindle_db::repo::preferences::load(conn, &read))
        .await
        .map(|prefs| prefs.game_detection_enabled)
        .map_err(|e| tracing::warn!(error = %e, "game detection: preferences unreadable"))
        .unwrap_or(false);
    if enabled {
        ctx.game.start(ctx, unlock, owner)?;
    }
    Ok(enabled)
}

#[cfg(test)]
#[path = "game_stage_tests.rs"]
mod tests;
