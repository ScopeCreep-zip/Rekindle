//! The owner side (plan RH1r-c O2–O4, RV1): rotate our General route on
//! evidence and republish it. Two tasks per node: [`run_rotations`] and
//! [`run_publisher`], the one writer of our personal route blob.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_records::port::RouteClass;
use rekindle_route::liveness::{PUBLISH_RETRY_JITTER_MS, PUBLISH_RETRY_MIN};
use tokio::sync::watch;
use tokio::time::Instant;

use super::Liveness;
use crate::shared::SharedState;

/// How often the rotation loop looks at the evidence.
const EVIDENCE_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// The session the publisher writes our blob into.
pub(crate) type SharedSession = Arc<parking_lot::RwLock<Option<crate::session::Session>>>;

/// Rotate our General route on inbound silence while a peer should be
/// answering us (O2, plan RV1), when a friend's DM chain runs ahead of the
/// hints that should have reached us over it, or when a friend re-sent a
/// call we receipted over it (plan RV2), until shutdown. A route Veilid
/// reports dead is `OwnRoutes::on_dead`'s.
pub(crate) async fn run_rotations(live: Arc<Liveness>, mut offers: Offers) {
    loop {
        // Biased, the end first: a closed `offers` (shutdown) beside a tick
        // rotates nothing more.
        tokio::select! {
            biased;
            changed = offers.changed() => if changed.is_err() { return },
            () = tokio::time::sleep(EVIDENCE_TICK) => {}
        }
        let now = Instant::now();
        if !live.track(now) {
            continue;
        }
        let expecting = live.expecting();
        let silent = live
            .general
            .lock()
            .as_ref()
            .is_some_and(|s| s.rules.silent(now, expecting));
        let ahead = live
            .general
            .lock()
            .as_ref()
            .is_some_and(|s| s.rules.chain_ahead(now));
        let lost = live
            .general
            .lock()
            .as_ref()
            .and_then(|s| s.rules.receipt_lost(now));
        if silent {
            live.rotate("inbound silence while a peer should answer");
        } else if ahead {
            live.rotate("a friend's DM chain ran ahead of its hints");
        } else if let Some(peer) = lost {
            // The peer's cooldown starts before the route changes.
            if let Some(state) = live.general.lock().as_mut() {
                state.rules.rotated_for(&peer, now);
            }
            live.rotate("a friend re-sent a call we receipted");
        }
    }
}

/// What the publisher waits on: the latest General blob to publish; the
/// tasks end when the route follower drops its sender.
pub(crate) type Offers = watch::Receiver<Option<Vec<u8>>>;

/// Publish each new General blob to the profile's route subkey and the
/// mailbox (the DHT gives each write the next seq): a rotation's
/// replacement is debounced, kept [`rekindle_route::liveness::PUBLISH_GAP`]
/// apart and retried after a miss; once one lands, the routes it replaced
/// are released (O3, O4).
pub(crate) async fn run_publisher(
    live: Arc<Liveness>,
    mut offers: Offers,
    session: SharedSession,
    shared: Arc<SharedState>,
) {
    loop {
        if offers.changed().await.is_err() {
            return;
        }
        let Some(blob) = offers.borrow_and_update().clone() else {
            continue;
        };
        let heal = live.rotated.swap(false, Ordering::SeqCst);
        live.track(Instant::now());
        // Only a rotation's republish is paced (O3): a route Veilid
        // replaced, a renew or the first blob goes at once.
        let now = Instant::now();
        let mut at = if heal { live.publish_at(now) } else { now };
        loop {
            // Biased, the newer blob first: a blob offered at the instant
            // the publish falls due supersedes the one about to be written.
            tokio::select! {
                biased;
                changed = offers.changed() => {
                    if changed.is_err() { return }
                    // A newer blob supersedes this one; keep the rotation mark.
                    if heal { live.rotated.store(true, Ordering::SeqCst); }
                    offers.mark_changed();
                    break;
                }
                () = tokio::time::sleep_until(at) => {}
            }
            let route_id =
                crate::route_trace::own_route_id(live.routes.as_ref(), RouteClass::General, &blob);
            match crate::broadcast::node::publish_personal_route(
                &session,
                &shared,
                &blob,
                route_id.as_deref(),
            )
            .await
            {
                None => break,
                Some(landed) => {
                    live.published(Instant::now());
                    if landed {
                        live.routes.release_retired_own(RouteClass::General);
                        break;
                    }
                    if !heal {
                        // The record pool re-pushes its held writes itself.
                        break;
                    }
                    let retry = PUBLISH_RETRY_MIN + live.jitter(PUBLISH_RETRY_JITTER_MS);
                    tracing::info!(
                        retry_ms = retry.as_millis(),
                        "replacement route publish missed; retrying"
                    );
                    at = Instant::now() + retry;
                }
            }
        }
    }
}

impl Liveness {
    /// When a publish asked for now may go.
    fn publish_at(&self, now: Instant) -> Instant {
        self.general
            .lock()
            .as_ref()
            .map_or(now, |s| s.rules.publish_at(now))
    }

    /// A publish went out at `now`.
    fn published(&self, now: Instant) {
        if let Some(state) = self.general.lock().as_mut() {
            state.rules.published(now);
        }
    }
}
