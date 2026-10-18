//! Our own routes on both hosts (plan C7.9d, E1.1): the route port
//! allocates, heals and releases them; this module is the one republisher
//! of a new personal route blob wherever peers read it, and backstops a
//! class whose allocation failed for good.

use std::sync::Arc;

use rekindle_records::port::{PeerTarget, RouteClass, RoutePort, RouteState};
use tokio::sync::mpsc;

use super::TransportNode;
use crate::error::{Result, TransportError};
use crate::handler::{InboundHandler, TransportEvent};
use crate::shared::SharedState;

impl TransportNode {
    /// A send target for `route_blob`, imported through the process's one
    /// route importer (plan C7.3, D4): an already-imported blob is a cache
    /// hit, never a second import.
    ///
    /// # Errors
    /// The blob does not decode or import.
    pub fn import_route(&self, route_blob: &[u8]) -> Result<PeerTarget> {
        self.ports
            .routes
            .import(route_blob)
            .map_err(|e| TransportError::RouteImportFailed {
                peer: String::new(),
                reason: e.to_string(),
            })
    }
}

/// Republish our personal route blob to the profile's route subkey and
/// the mailbox. Without an unlocked session, or before its own-records
/// stage has the keys, there is nothing to write to: that stage publishes
/// the then-current blob itself (`rekindle_node::host::own_records`).
async fn publish_personal_route(
    session: &Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    shared: &Arc<SharedState>,
    blob: &[u8],
) {
    let (profile_key, mailbox_key) = {
        let guard = session.read();
        match guard.as_ref() {
            Some(s) => (
                s.identity.profile_dht_key.clone(),
                s.identity.mailbox_dht_key.clone(),
            ),
            None => return,
        }
    };
    let Some(pool) = shared.records() else {
        return;
    };
    if !profile_key.is_empty() {
        match rekindle_protocol::dht::profile::set_own_profile_subkey(
            pool.as_ref(),
            &profile_key,
            crate::payload::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
            blob.to_vec(),
        )
        .await
        {
            Ok(outcome) if !outcome.missed() => {}
            Ok(outcome) => {
                tracing::warn!(
                    ?outcome,
                    "personal route: profile republish not at consensus"
                );
            }
            Err(e) => tracing::warn!(error = %e, "personal route: profile republish failed"),
        }
    }
    if !mailbox_key.is_empty() {
        match rekindle_protocol::dht::mailbox::update_mailbox_route(
            pool.as_ref(),
            &mailbox_key,
            blob,
        )
        .await
        {
            Ok(outcome) if !outcome.missed() => {}
            Ok(outcome) => {
                tracing::warn!(
                    ?outcome,
                    "personal route: mailbox republish not at consensus"
                );
            }
            Err(e) => tracing::warn!(error = %e, "personal route: mailbox republish failed"),
        }
    }
    tracing::info!(blob_len = blob.len(), "personal route republished");
}

/// Told when a route changed (the handler's network status), when the node
/// dispatches its own events; the adopted node has no handler until E5.7.
pub(super) type OnRoutesChanged =
    Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>;

/// [`OnRoutesChanged`] for `handler`.
pub(super) fn notify_handler<H: InboundHandler>(handler: Arc<H>) -> OnRoutesChanged {
    Box::new(move || {
        let handler = Arc::clone(&handler);
        Box::pin(async move { handler.on_event(TransportEvent::RoutesChanged).await })
    })
}

/// Follow our routes until shutdown, on both hosts (plan E1.1: the one
/// republisher): every change is reported, a new general blob is
/// republished to the session's profile and mailbox; the watchdog tick wants
/// both classes again, a no-op while a route is live or allocating, so a
/// class whose allocation failed for good is retried. Routes are wanted from
/// node-ready, unlocked or not (C7.9b). They are never rotated on a timer
/// (veilid-core tests their health and reports death in `RouteChange`).
pub(super) async fn run_route_publisher(
    on_change: Option<OnRoutesChanged>,
    routes: Arc<dyn RoutePort>,
    watchdog_secs: u64,
    mut shutdown_rx: mpsc::Receiver<()>,
    session: Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    shared: Arc<SharedState>,
) {
    let mut general = routes.own_state(RouteClass::General);
    let mut media = routes.own_state(RouteClass::Media);
    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(watchdog_secs.max(1)));
    interval.tick().await; // skip immediate first tick
    loop {
        tokio::select! {
            changed = general.changed() => {
                if changed.is_err() {
                    break;
                }
                let state = general.borrow_and_update().clone();
                if let Some(on_change) = &on_change {
                    on_change().await;
                }
                if let RouteState::Available { blob } = state {
                    publish_personal_route(&session, &shared, &blob).await;
                }
            }
            changed = media.changed() => {
                if changed.is_err() {
                    break;
                }
                media.borrow_and_update();
                if let Some(on_change) = &on_change {
                    on_change().await;
                }
            }
            _ = interval.tick() => routes.want_own(),
            _ = shutdown_rx.recv() => {
                tracing::info!("route publisher shutting down");
                break;
            }
        }
    }
}
