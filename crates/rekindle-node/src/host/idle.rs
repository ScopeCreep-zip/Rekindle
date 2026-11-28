//! The idle stage (plan E1.1), both hosts: every 30 s the idle time from
//! the Host's idle port (`HostConfig::idle`: the OS's in production, the
//! simulator's in the netsim, plan E2.B), against the identity's
//! `auto_away_minutes`, read from its database each tick (the host holds no
//! copy of the preferences). Where no compositor or display reports idle
//! time, the port reads `None`: the host reports `IdleState::Unavailable`
//! once and auto-away does not arm.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::own_presence::OwnPresence;

/// How often the idle time is read.
const TICK: Duration = Duration::from_secs(30);

/// What the idle stage reads and reports to.
pub(crate) struct IdleStage {
    pub presence: OwnPresence,
    pub db: Arc<rekindle_db::DbHandle>,
    pub source: Arc<dyn rekindle_utils::IdleSource>,
    pub owner: String,
}

impl IdleStage {
    /// Run until `stop`.
    pub(crate) async fn run(self, stop: CancellationToken) {
        let Self {
            presence,
            db,
            source,
            owner,
        } = self;
        run(presence, db, source, owner, stop).await;
    }
}

async fn run(
    presence: OwnPresence,
    db: Arc<rekindle_db::DbHandle>,
    source: Arc<dyn rekindle_utils::IdleSource>,
    owner: String,
    stop: CancellationToken,
) {
    let mut interval = tokio::time::interval(TICK);
    loop {
        // Biased, stop first: a lock that lands with a tick reads no idle
        // time and changes no presence (the netsim's order follows its seed).
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            _ = interval.tick() => {}
        }
        let read = Arc::clone(&source);
        let idle_secs = match rekindle_lifecycle::run_blocking(move || read.idle_seconds()).await {
            Ok(Some(secs)) => secs,
            Ok(None) => {
                presence.idle_unavailable();
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "idle time read failed");
                continue;
            }
        };
        let Some(minutes) = auto_away_minutes(&db, &owner).await else {
            continue;
        };
        apply(&presence, Some(idle_secs), minutes);
    }
}

/// One idle reading against `minutes` (0 = off): at or past the threshold,
/// auto-away; under it, activity, which ends an auto-away whose reason went;
/// no reading, idle is unavailable, which ends one too (`OwnPresence`).
pub(crate) fn apply(presence: &OwnPresence, idle_secs: Option<u64>, minutes: u32) {
    match idle_secs {
        Some(secs) if minutes > 0 && secs >= u64::from(minutes) * 60 => presence.auto_away(),
        Some(_) => presence.active(),
        None => presence.idle_unavailable(),
    }
}

/// Read the idle time once and apply `minutes` now (`SetAutoAway`): the
/// change takes effect at once, not at the next tick.
pub(crate) async fn apply_now(
    presence: &OwnPresence,
    source: &Arc<dyn rekindle_utils::IdleSource>,
    minutes: u32,
) {
    let read = Arc::clone(source);
    match rekindle_lifecycle::run_blocking(move || read.idle_seconds()).await {
        Ok(idle_secs) => apply(presence, idle_secs, minutes),
        Err(e) => {
            tracing::warn!(error = %e, "idle time read failed");
            if minutes == 0 {
                presence.active();
            }
        }
    }
}

/// The identity's auto-away threshold, minutes (0 = off).
async fn auto_away_minutes(db: &rekindle_db::DbHandle, owner: &str) -> Option<u32> {
    let db = db.current().ok()?;
    let owner = owner.to_owned();
    match db
        .call(move |conn| rekindle_db::repo::preferences::load(conn, &owner))
        .await
    {
        Ok(prefs) => Some(prefs.auto_away_minutes),
        Err(e) => {
            tracing::warn!(error = %e, "idle stage: preferences unreadable");
            None
        }
    }
}

#[cfg(test)]
#[path = "idle_tests.rs"]
mod tests;
