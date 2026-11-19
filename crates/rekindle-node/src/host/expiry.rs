//! The disappearing-message expiry task (plan E7.1): spawned in the unlock
//! `SessionScope`, it sleeps until the earliest `messages.expire_at` and is
//! woken (`FriendStores::expiry`) when a send or a display sets an earlier
//! one; never a poll. Each pass is
//! `rekindle_transport::operations::dm::disappearing::expire_due`.

use std::time::Duration;

use rekindle_transport::operations::dm::disappearing::expire_due;
use rekindle_transport::operations::friend::FriendHost;
use tokio_util::sync::CancellationToken;

/// Run until `stop`: delete what is due, then wait for the next deadline
/// or a wake.
pub(crate) async fn run(host: impl FriendHost, stop: CancellationToken) {
    loop {
        let next = match expire_due(&host).await {
            Ok(next) => next,
            Err(e) => {
                // Locked or the storage failed: wait for a wake (the next
                // send or display) rather than spin.
                tracing::warn!(error = %e, "disappearing-message expiry pass failed");
                None
            }
        };
        let wait = next.map(|at| {
            let ms = at.saturating_sub(host.clock().now_ms_i64()).max(0);
            Duration::from_millis(u64::try_from(ms).unwrap_or(0))
        });
        let woken = host.stores().expiry.notified();
        tokio::select! {
            () = stop.cancelled() => return,
            () = woken => {}
            () = sleep(wait) => {}
        }
    }
}

/// `wait`, or forever with no deadline.
async fn sleep(wait: Option<Duration>) {
    match wait {
        Some(wait) => tokio::time::sleep(wait).await,
        None => std::future::pending().await,
    }
}
