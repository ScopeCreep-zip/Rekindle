//! Starting and stopping the per-community presence polls.
//!
//! Polls begin on unlock, not at daemon start: before unlock there is no
//! signing key, so a poll could neither sign its own presence row nor
//! decrypt anyone else's session extras.
//!
//! The request rides a channel for the same reason MEK rotation does —
//! `handle_unlock` holds `&DaemonContext` and the daemon's single `Arc`
//! stops at `dispatch`, while `start_presence_poll` needs an owned
//! `Arc<D>` for a loop that outlives the request. One supervisor owning
//! the `Arc` is cheaper than threading it through 68 handler signatures.

use std::sync::Arc;

use crate::daemon::dispatch::DaemonContext;

use super::DaemonPresenceAdapter;

/// Communities whose presence poll should start.
#[derive(Debug, Clone)]
pub struct PresenceStartRequest {
    pub community_ids: Vec<String>,
}

pub type PresenceStartSender = tokio::sync::mpsc::UnboundedSender<PresenceStartRequest>;
pub type PresenceStartReceiver = tokio::sync::mpsc::UnboundedReceiver<PresenceStartRequest>;

/// Create the unlock/supervisor channel pair.
#[must_use]
pub fn channel() -> (PresenceStartSender, PresenceStartReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Drain start requests and spawn one poll per community.
pub async fn run_supervisor(ctx: Arc<DaemonContext>, mut rx: PresenceStartReceiver) {
    tracing::info!("presence supervisor started");
    while let Some(request) = rx.recv().await {
        for community_id in request.community_ids {
            // A fresh adapter per community: it is one `Arc` clone of the
            // context, and giving each poll its own avoids a shared
            // handle outliving a community that gets left.
            let adapter = Arc::new(DaemonPresenceAdapter::new(Arc::clone(&ctx)));
            tracing::debug!(
                community = %&community_id[..16.min(community_id.len())],
                "starting presence poll"
            );
            rekindle_presence::community::start_presence_poll(adapter, community_id);
        }
    }
    tracing::info!("presence supervisor stopped");
}
