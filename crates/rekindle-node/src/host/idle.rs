//! The idle stage (plan E1.1), both hosts: every 30 s the OS idle time,
//! against the identity's `auto_away_minutes`, read from its database each
//! tick (the host holds no copy of the preferences). Where no compositor or
//! display reports idle time, `rekindle_idle::get_idle_seconds()` is
//! `None`: the host reports `IdleState::Unavailable` once and auto-away
//! does not arm.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::own_presence::OwnPresence;

/// How often the idle time is read.
const TICK: Duration = Duration::from_secs(30);

/// Run until `stop`.
pub(crate) async fn run(
    presence: OwnPresence,
    db: Arc<rekindle_db::DbHandle>,
    owner: String,
    stop: CancellationToken,
) {
    let mut interval = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            () = stop.cancelled() => return,
        }
        let idle_secs =
            match rekindle_lifecycle::run_blocking(rekindle_idle::get_idle_seconds).await {
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
        if minutes > 0 && idle_secs >= u64::from(minutes) * 60 {
            presence.auto_away();
        } else {
            presence.active();
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
