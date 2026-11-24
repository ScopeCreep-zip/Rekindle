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
    /// The blob is a peer route known dead (`RouteUnusable`, plan RV1g B1),
    /// or it does not decode or import.
    pub fn import_route(&self, route_blob: &[u8]) -> Result<PeerTarget> {
        import_live(self.ports.routes.as_ref(), &self.liveness, route_blob)
    }
}

/// Import `route_blob` unless it is a peer route known dead (plan RV1g
/// B1): Veilid's import is idempotent (`route_remote.rs:21-28`), so a dead
/// blob re-read from an unchanged record would be imported, tested and
/// dropped again on every attempt. A refused blob is `RouteUnusable`, the
/// verdict a send into it would have had; nothing is released.
pub(crate) fn import_live(
    routes: &dyn RoutePort,
    liveness: &crate::liveness::Liveness,
    route_blob: &[u8],
) -> Result<PeerTarget> {
    if liveness.route_declared_dead(route_blob) {
        return Err(TransportError::RouteUnusable {
            target: crate::route_trace::blob_tag(route_blob),
            reason: "declared dead".into(),
        });
    }
    routes
        .import(route_blob)
        .map_err(|e| TransportError::RouteImportFailed {
            peer: String::new(),
            reason: e.to_string(),
        })
}

/// Republish our personal route blob to the profile's route subkey and
/// the mailbox: `Some(true)` when both landed, `Some(false)` when one
/// missed (the liveness publisher retries it, plan RH1r-c O3). Without an
/// unlocked session, or before its own-records stage has the keys, there
/// is nothing to write to (`None`): that stage publishes the then-current
/// blob itself (`rekindle_node::host::own_records`). `route_id` is the
/// General route's id while `blob` is still its blob, for the trace line
/// (`crate::route_trace`, plan RD2).
pub(crate) async fn publish_personal_route(
    session: &Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    shared: &Arc<SharedState>,
    blob: &[u8],
    route_id: Option<&str>,
) -> Option<bool> {
    let (profile_key, mailbox_key) = {
        let guard = session.read();
        match guard.as_ref() {
            Some(s) => (
                s.identity.profile_dht_key.clone(),
                s.identity.mailbox_dht_key.clone(),
            ),
            None => return None,
        }
    };
    if profile_key.is_empty() && mailbox_key.is_empty() {
        return None;
    }
    let pool = shared.records()?;
    let mut landed = true;
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
                landed = false;
                tracing::warn!(
                    ?outcome,
                    "personal route: profile republish not at consensus"
                );
            }
            Err(e) => {
                landed = false;
                tracing::warn!(error = %e, "personal route: profile republish failed");
            }
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
                landed = false;
                tracing::warn!(
                    ?outcome,
                    "personal route: mailbox republish not at consensus"
                );
            }
            Err(e) => {
                landed = false;
                tracing::warn!(error = %e, "personal route: mailbox republish failed");
            }
        }
    }
    crate::route_trace::published(RouteClass::General, route_id, blob, landed);
    Some(landed)
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

/// Follow our routes until shutdown, on both hosts (plan E1.1): every
/// change is reported, and a new general blob is offered to the one
/// republisher (`liveness::owner::run_publisher`, plan RH1r-c), which
/// paces it into the session's profile and mailbox; dropping `offers` at
/// shutdown ends it and the self-tests. The watchdog tick wants
/// both classes again, a no-op while a route is live or allocating, so a
/// class whose allocation failed for good is retried. Routes are wanted from
/// node-ready, unlocked or not (C7.9b). They are never rotated on a timer;
/// a route Veilid reports dead in `RouteChange` is replaced by `OwnRoutes`,
/// and one our self-test finds dead by the liveness tasks (RH1r-c).
pub(super) async fn run_route_publisher(
    on_change: Option<OnRoutesChanged>,
    routes: Arc<dyn RoutePort>,
    watchdog_secs: u64,
    mut shutdown_rx: mpsc::Receiver<()>,
    offers: Vec<tokio::sync::watch::Sender<Option<Vec<u8>>>>,
) {
    let mut general = routes.own_state(RouteClass::General);
    let mut media = routes.own_state(RouteClass::Media);
    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(watchdog_secs.max(1)));
    interval.tick().await; // skip immediate first tick
    loop {
        // Biased, shutdown first: a shutdown beside a route change offers
        // no blob to a stopping republisher. General before Media keeps
        // two changes at one instant in one order; none of the arms is
        // ready again until its source changes or the interval's period.
        tokio::select! {
            biased;
            _ = shutdown_rx.recv() => {
                tracing::info!("route publisher shutting down");
                break;
            }
            changed = general.changed() => {
                if changed.is_err() {
                    break;
                }
                let state = general.borrow_and_update().clone();
                if let Some(on_change) = &on_change {
                    on_change().await;
                }
                // A withdrawal publishes the empty route (plan RV1g):
                // readers take an empty blob as no route.
                let blob = match state {
                    RouteState::Available { blob } => Some(blob),
                    RouteState::Withdrawn => Some(Vec::new()),
                    _ => None,
                };
                if let Some(blob) = blob {
                    // The publisher's first, then the self-tests': one
                    // order every run (RK1).
                    for offer in &offers {
                        offer.send_replace(Some(blob.clone()));
                    }
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
        }
    }
}
