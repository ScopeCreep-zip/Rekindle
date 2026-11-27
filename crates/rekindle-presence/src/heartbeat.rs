//! The session's one STATUS publisher (plan C7.8c).
//!
//! Every change of our status (a manual pick, auto-away and its restore,
//! the tray, an identity rotation) and the periodic heartbeat go through
//! this loop, which writes the identity's *current* status at write time.
//! Two producers each writing their own earlier read let the stale one land
//! last (observed: a heartbeat's "Online" beat a concurrent auto-away's
//! "Away"); with one writer reading at write time it cannot. veilid-core
//! already serializes writes to a subkey, so the ordering bug was ours.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::interval;

use crate::deps::StatusPublisherDeps;
use crate::friend::publish_status;
use crate::status::UserStatusKind;

/// Heartbeat cadence (60 s): readers treat a status older than
/// `STALE_PRESENCE_THRESHOLD_MS` (150 s) as offline.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 60;

/// What woke the publisher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// A status change: publish whatever the status now is.
    Change,
    /// The heartbeat: re-publish a shown status.
    Tick,
}

/// Whether `status` is one peers see as offline, the one rule every
/// presence channel follows (the STATUS beat here, the Host's community
/// gossip, the desktop fan-out). Invisible publishes
/// exactly what Offline publishes (the same wire byte) and, like a real
/// Offline, writes it once and then nothing until the status changes, so
/// no watcher or replica can tell the two apart by a later beat.
#[must_use]
pub const fn hides_presence(status: UserStatusKind) -> bool {
    matches!(status, UserStatusKind::Offline | UserStatusKind::Invisible)
}

/// Publish the current status, read now. `None` when there is
/// nothing to publish: logged out, or a hidden status once an offline
/// write has landed (`published` is the last status written successfully).
/// A hidden status whose write failed is retried on the next tick.
fn status_to_publish(
    current: Option<UserStatusKind>,
    published: Option<UserStatusKind>,
) -> Option<UserStatusKind> {
    let status = current?;
    (!(hides_presence(status) && published.is_some_and(hides_presence))).then_some(status)
}

/// Run the publisher until `stop`. `wake` is notified on every status
/// change; it keeps one permit, so a change during a publish publishes
/// once more after it. A failed write is retried by the next wake or tick.
pub async fn run_status_publisher<D: StatusPublisherDeps + ?Sized>(
    deps: Arc<D>,
    wake: Arc<Notify>,
    stop: tokio_util::sync::CancellationToken,
) {
    let mut tick = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    tick.tick().await; // skip the immediate first fire
    let mut published = None;

    loop {
        // Biased, stop first: a stop beside a wake publishes nothing more.
        // A change and a beat at once publish the same current status, so
        // either order writes the same; the change is named first.
        let reason = tokio::select! {
            biased;
            () = stop.cancelled() => {
                tracing::debug!("status publisher shutting down");
                return;
            }
            () = wake.notified() => Wake::Change,
            _ = tick.tick() => Wake::Tick,
        };
        let Some(status) = status_to_publish(deps.current_identity_status(), published) else {
            continue;
        };
        match publish_status(Arc::clone(&deps), status).await {
            Ok(()) => published = Some(status),
            Err(error) => {
                published = None;
                tracing::debug!(%error, ?reason, "status publish failed; the next wake retries");
            }
        }
    }
}

#[cfg(test)]
#[path = "heartbeat_tests.rs"]
mod tests;
