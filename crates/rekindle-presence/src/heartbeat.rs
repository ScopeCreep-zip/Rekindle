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
    /// The heartbeat: re-publish unless we are Offline.
    Tick,
}

/// Publish for `wake`: the current status, read now. `None` when there is
/// nothing to publish (logged out, or Offline on a heartbeat).
fn status_to_publish(current: Option<UserStatusKind>, wake: Wake) -> Option<UserStatusKind> {
    match (current?, wake) {
        (UserStatusKind::Offline, Wake::Tick) => None,
        (status, _) => Some(status),
    }
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

    loop {
        let reason = tokio::select! {
            () = stop.cancelled() => {
                tracing::debug!("status publisher shutting down");
                return;
            }
            () = wake.notified() => Wake::Change,
            _ = tick.tick() => Wake::Tick,
        };
        let Some(status) = status_to_publish(deps.current_identity_status(), reason) else {
            continue;
        };
        if let Err(error) = publish_status(Arc::clone(&deps), status).await {
            tracing::debug!(%error, ?reason, "status publish failed; the next wake retries");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_publishes_any_status_and_a_tick_skips_offline() {
        use UserStatusKind::{Away, Offline, Online};
        assert_eq!(status_to_publish(Some(Away), Wake::Change), Some(Away));
        assert_eq!(
            status_to_publish(Some(Offline), Wake::Change),
            Some(Offline)
        );
        assert_eq!(status_to_publish(Some(Online), Wake::Tick), Some(Online));
        assert_eq!(status_to_publish(Some(Offline), Wake::Tick), None);
        assert_eq!(status_to_publish(None, Wake::Change), None, "logged out");
    }

    /// The bug this loop exists for: a change racing a heartbeat ends with
    /// the newer status written last, because each publish reads the
    /// status at write time.
    #[tokio::test]
    async fn a_change_during_a_heartbeat_ends_with_the_newer_status() {
        use std::sync::atomic::{AtomicU8, Ordering};

        use async_trait::async_trait;
        use parking_lot::Mutex;

        use crate::deps::PresenceError;

        struct Deps {
            status: AtomicU8,
            written: Mutex<Vec<u8>>,
        }
        #[async_trait]
        impl StatusPublisherDeps for Deps {
            fn profile_dht_info(&self) -> Option<String> {
                Some("VLD0:profile".into())
            }
            async fn write_profile_status_subkey(
                &self,
                _key: &str,
                payload: Vec<u8>,
            ) -> Result<(), PresenceError> {
                tokio::task::yield_now().await;
                self.written.lock().push(payload[0]);
                Ok(())
            }
            fn current_identity_status(&self) -> Option<UserStatusKind> {
                Some(if self.status.load(Ordering::SeqCst) == 0 {
                    UserStatusKind::Online
                } else {
                    UserStatusKind::Away
                })
            }
            fn now_ms(&self) -> i64 {
                0
            }
        }
        let deps = Arc::new(Deps {
            status: AtomicU8::new(0),
            written: Mutex::new(Vec::new()),
        });
        let wake = Arc::new(Notify::new());
        let stop = tokio_util::sync::CancellationToken::new();
        let task = tokio::spawn(run_status_publisher(
            Arc::clone(&deps),
            Arc::clone(&wake),
            stop.clone(),
        ));
        wake.notify_one(); // a publish of Online is under way…
        deps.status.store(1, Ordering::SeqCst); // …when the user goes Away
        wake.notify_one();
        tokio::time::sleep(Duration::from_millis(50)).await;
        stop.cancel();
        task.await.unwrap();
        let written = deps.written.lock().clone();
        assert_eq!(
            written.last().copied(),
            Some(crate::friend::status_to_wire_byte(UserStatusKind::Away)),
            "written: {written:?}"
        );
    }
}
