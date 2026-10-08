//! Our own routes on the daemon track (plan C7.9d): `OwnRoutes`
//! (rekindle-protocol) allocates, heals and releases them; this module
//! republishes a new personal route blob wherever peers read it, and
//! backstops a class whose allocation failed for good.

use std::sync::Arc;

use rekindle_protocol::own_routes::{OwnRoutes, RouteClass, RouteState, VeilidRouteAllocator};
use tokio::sync::mpsc;

use super::TransportNode;
use crate::broadcast::peer_registry::PeerTarget;
use crate::error::{Result, TransportError};
use crate::handler::{InboundHandler, TransportEvent};
use crate::shared::SharedState;

impl TransportNode {
    /// A send target for `route_blob`, imported through the process's one
    /// route importer (plan C7.3, D4): an already-imported blob is a cache
    /// hit, never a second import.
    pub fn import_route(&self, route_blob: &[u8]) -> Result<PeerTarget> {
        let route_id = self.route_imports.get_or_import(route_blob).map_err(|e| {
            TransportError::RouteImportFailed {
                peer: String::new(),
                reason: format!("{e}"),
            }
        })?;
        Ok(PeerTarget { route_id })
    }
}

/// Republish our personal route blob to the profile's route subkey and
/// the mailbox. Without an unlocked session there is nothing to write to:
/// the next unlock's resume publishes the then-current blob (it reads it
/// after holding the records, so a blob landing mid-resume is not lost).
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
            &pool,
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
        match rekindle_protocol::dht::mailbox::update_mailbox_route(&pool, &mailbox_key, blob).await
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

/// Follow our routes until shutdown: every change is reported to the
/// handler (the window's network status), a new general blob is republished;
/// the watchdog tick wants both classes again, a no-op while a route is
/// live or allocating, so a class whose allocation failed for good is
/// retried. Routes are never rotated on a timer (veilid-core tests their
/// health and reports death in `RouteChange`).
pub(super) async fn run_route_publisher<H: InboundHandler>(
    handler: Arc<H>,
    own_routes: Arc<OwnRoutes<VeilidRouteAllocator>>,
    watchdog_secs: u64,
    mut shutdown_rx: mpsc::Receiver<()>,
    session: Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    shared: Arc<SharedState>,
) {
    let mut general = own_routes.state(RouteClass::General);
    let mut media = own_routes.state(RouteClass::Media);
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
                handler.on_event(TransportEvent::RoutesChanged).await;
                if let RouteState::Available { blob } = state {
                    publish_personal_route(&session, &shared, &blob).await;
                }
            }
            changed = media.changed() => {
                if changed.is_err() {
                    break;
                }
                media.borrow_and_update();
                handler.on_event(TransportEvent::RoutesChanged).await;
            }
            _ = interval.tick() => {
                // Only an unlocked session wants routes (plan C7.9d).
                if session.read().is_some() && shared.records().is_some() {
                    own_routes.want(RouteClass::General);
                    own_routes.want(RouteClass::Media);
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("route publisher shutting down");
                break;
            }
        }
    }
}
