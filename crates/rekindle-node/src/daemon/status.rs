//! The daemon's one STATUS publisher (plan C7.8c): the shared loop
//! (`rekindle_presence::run_status_publisher`) over the unlock's current
//! status, so status changes and the 60 s heartbeat never race, and friends
//! do not read a daemon user as stale-offline 150 s after a change. Lock
//! writes Offline, bounded, as the desktop's logout does.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_presence::{PresenceError, StatusPublisherDeps, UserStatusKind};
use rekindle_transport::TransportNode;

use super::dispatch::DaemonContext;

/// How long lock waits for the Offline write; the write itself runs on
/// (Signal Desktop bounds its shutdown queue drain the same way).
const OFFLINE_WRITE_WAIT: Duration = Duration::from_secs(10);

/// What the publisher needs: our profile, the unlock's status, the pool.
struct DaemonStatusDeps {
    node: Arc<TransportNode>,
    profile_key: String,
    status: Arc<RwLock<UserStatusKind>>,
}

#[async_trait]
impl StatusPublisherDeps for DaemonStatusDeps {
    fn profile_dht_info(&self) -> Option<String> {
        (!self.profile_key.is_empty()).then(|| self.profile_key.clone())
    }

    async fn write_profile_status_subkey(
        &self,
        profile_key: &str,
        payload: Vec<u8>,
    ) -> Result<(), PresenceError> {
        let pool = self
            .node
            .records()
            .ok_or_else(|| PresenceError::Dht("record pool not running".into()))?;
        let outcome =
            rekindle_protocol::dht::profile::set_own_profile_status(&pool, profile_key, payload)
                .await
                .map_err(|e| PresenceError::Dht(e.to_string()))?;
        if outcome.missed() {
            return Err(PresenceError::Dht(format!(
                "status not stored ({outcome:?})"
            )));
        }
        Ok(())
    }

    fn current_identity_status(&self) -> Option<UserStatusKind> {
        Some(*self.status.read())
    }

    fn now_ms(&self) -> i64 {
        rekindle_utils::timestamp_ms_i64()
    }
}

fn deps(ctx: &DaemonContext) -> Option<Arc<DaemonStatusDeps>> {
    let node = ctx.transport.read().clone()?;
    let profile_key = ctx
        .session
        .read()
        .as_ref()
        .map(|s| s.identity.profile_dht_key.clone())?;
    Some(Arc::new(DaemonStatusDeps {
        node,
        profile_key,
        status: Arc::clone(&ctx.status),
    }))
}

/// Start the publisher in the unlock scope; the unlock starts Online.
pub(crate) fn start(ctx: &DaemonContext, scope: &Arc<rekindle_lifecycle::SessionScope>) {
    *ctx.status.write() = UserStatusKind::Online;
    let Some(deps) = deps(ctx) else {
        return;
    };
    let wake = Arc::clone(&ctx.status_wake);
    scope.spawn_with_token_or_drop("status publisher", move |stop| {
        rekindle_presence::run_status_publisher(deps, wake, stop)
    });
    // Resume opened the profile: publish the unlock's status now.
    ctx.status_wake.notify_one();
}

/// Set the unlock's status and have the publisher write it.
pub(crate) fn set(ctx: &DaemonContext, status: UserStatusKind) {
    *ctx.status.write() = status;
    ctx.status_wake.notify_one();
}

/// Lock: write Offline once the unlock's tasks (the publisher with them)
/// have stopped, waiting at most [`OFFLINE_WRITE_WAIT`].
pub(crate) async fn publish_offline(ctx: &DaemonContext) {
    // No pool (a failed unlock, or already locked): nothing was published.
    let Some(deps) = deps(ctx).filter(|d| d.node.records().is_some()) else {
        return;
    };
    *ctx.status.write() = UserStatusKind::Offline;
    match tokio::time::timeout(
        OFFLINE_WRITE_WAIT,
        rekindle_presence::publish_status(deps, UserStatusKind::Offline),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, "failed to publish offline status"),
        Err(_) => tracing::info!("offline status still publishing; lock continues"),
    }
}
