//! The owner side (plan RH1r-c O1–O5, RT1): self-test our own routes and
//! republish a replacement. Two tasks per node: [`run_self_tests`] and
//! [`run_publisher`], the one writer of our personal route blob.

use std::collections::HashSet;
use std::sync::Arc;

use rekindle_records::port::OwnPath;
use rekindle_records::port::RouteClass;
use rekindle_route::liveness::{
    resumed_across, test_discarded, SelfTest, PROBE_TIMEOUT, PUBLISH_RETRY_JITTER_MS,
    PUBLISH_RETRY_MIN, RETEST_JITTER_MS, SELF_TEST_JITTER_MS, SILENCE_TRIGGER,
};
use tokio::sync::watch;
use tokio::time::Instant;

use super::{Liveness, TESTED};
use crate::shared::SharedState;

/// How often the self-test loop looks at its schedule.
const SELF_TEST_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// The session the publisher writes our blob into.
pub(crate) type SharedSession = Arc<parking_lot::RwLock<Option<crate::session::Session>>>;

impl Liveness {
    /// One round trip through our route of `class` (O1, O6): `true` when
    /// our own probe came back within [`PROBE_TIMEOUT`]; `None` when the
    /// test spanned a suspend and is discarded, never counted (LF7, lead
    /// decision 45): the test after the resume decides.
    pub(crate) async fn self_test(&self, class: RouteClass) -> Option<bool> {
        let (nonce, datagram) = self.probe();
        self.outstanding.lock().insert(nonce);
        let profile = rekindle_route::contexts::RouteContextSpec::rc_safe().safety_profile();
        // General every time from outside, as a peer's call travels (LF9):
        // a loopback passes while strangers cannot reach the route
        // (veilid-core 0.5.8 `route_compile.rs:420-434`, `rpc_route.rs:329-365`).
        let path = if class == RouteClass::General {
            OwnPath::Outside
        } else {
            OwnPath::Loopback
        };
        let before = self.wall_ms();
        let started = Instant::now();
        let answer = tokio::time::timeout(
            PROBE_TIMEOUT,
            self.messages
                .call_own(class, path, &profile, datagram.clone()),
        )
        .await;
        self.outstanding.lock().remove(&nonce);
        let ok = matches!(&answer, Ok(Ok(reply)) if *reply == datagram);
        let after = self.wall_ms();
        if resumed_across(before, after, PROBE_TIMEOUT) {
            self.resumed();
        }
        if test_discarded(before, after, started.elapsed(), PROBE_TIMEOUT) {
            tracing::info!(
                ?class,
                ?path,
                ok,
                "own route self-test spanned a suspend; discarded"
            );
            return None;
        }
        tracing::debug!(?class, ?path, ok, "own route self-test");
        self.routes.self_tested_own(class, ok);
        Some(ok)
    }

    /// Whether `class` is due a self-test at `now`.
    fn due(&self, class: RouteClass, now: Instant) -> bool {
        let expecting = class == RouteClass::General && self.expecting();
        let mut classes = self.classes.lock();
        let Some(state) = classes.get_mut(&class) else {
            return false;
        };
        // Media is tested only while a call carries media over it.
        if class == RouteClass::Media
            && state
                .heard
                .is_none_or(|at| now.duration_since(at) >= SILENCE_TRIGGER)
        {
            return false;
        }
        state
            .rules
            .should_test(now, expecting, || self.jitter(SELF_TEST_JITTER_MS))
    }

    /// Record the test of `class` that ran at `now`.
    fn record(&self, class: RouteClass, now: Instant, ok: bool) -> SelfTest {
        let period = self.jitter(SELF_TEST_JITTER_MS);
        let retest = self.jitter(RETEST_JITTER_MS);
        self.classes
            .lock()
            .get_mut(&class)
            .map_or(SelfTest::Healthy, |s| {
                s.rules.tested(now, ok, period, retest)
            })
    }
}

/// Self-test our own routes until shutdown (O1, O2, O5); a dead one is
/// replaced, and the replacement is tested before it is published (O3) or,
/// for Media and Sending, before the old one is released (O4).
pub(crate) async fn run_self_tests(live: Arc<Liveness>, mut offers: Offers) {
    let mut healing: HashSet<RouteClass> = HashSet::new();
    loop {
        // Biased, the end first: a closed `offers` (shutdown) beside a tick
        // tests nothing more. Every other arm wakes the same pass, and none
        // is ready again until its next change or tick.
        let before = live.wall_ms();
        tokio::select! {
            biased;
            changed = offers.changed() => if changed.is_err() { return },
            () = live.network_changed.notified() => {}
            () = tokio::time::sleep(SELF_TEST_TICK) => {}
        }
        // A tick wait that the wall clock saw last far longer: a suspend.
        if resumed_across(before, live.wall_ms(), SELF_TEST_TICK) {
            live.resumed();
        }
        for class in TESTED {
            let now = Instant::now();
            if !live.track(class, now) {
                continue;
            }
            if class != RouteClass::General && healing.remove(&class) {
                // The Media or Sending replacement is live: test it, then
                // release the old one (no record names either route).
                match live.self_test(class).await {
                    Some(true) => live.routes.release_retired_own(class),
                    Some(false) => {
                        live.routes.replace_own_class(class);
                        healing.insert(class);
                    }
                    // Discarded: test the replacement again (LF7).
                    None => {
                        healing.insert(class);
                    }
                }
                continue;
            }
            if !live.due(class, now) {
                continue;
            }
            let Some(ok) = live.self_test(class).await else {
                continue;
            };
            match live.record(class, Instant::now(), ok) {
                SelfTest::Dead => {
                    tracing::warn!(?class, "own route failed its self-tests; replacing it");
                    if class == RouteClass::General {
                        live.heal_pending.lock().insert(class);
                    } else {
                        healing.insert(class);
                    }
                    live.routes.replace_own_class(class);
                }
                SelfTest::Retry => tracing::info!(?class, "own route self-test failed"),
                SelfTest::Healthy => {}
            }
        }
    }
}

/// What the publisher waits on: the latest General blob to publish; the
/// tasks end when the route follower drops its sender.
pub(crate) type Offers = watch::Receiver<Option<Vec<u8>>>;

/// Publish each new General blob to the profile's route subkey and the
/// mailbox (the DHT gives each write the next seq): a replacement from a
/// failed self-test is tested first, and a republish is debounced, kept
/// [`rekindle_route::liveness::PUBLISH_GAP`] apart and retried after a
/// miss; once one lands, the routes it replaced are released (O3, O4).
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
        let heal = live.heal_pending.lock().remove(&RouteClass::General);
        let tested = if heal && live.self_tests {
            live.self_test(RouteClass::General).await
        } else {
            Some(true)
        };
        if tested.is_none() {
            // Discarded across a suspend (LF7): test this blob again.
            live.heal_pending.lock().insert(RouteClass::General);
            offers.mark_changed();
            continue;
        }
        if tested == Some(false) {
            tracing::warn!("replacement route failed its self-test; replacing it again");
            live.heal_pending.lock().insert(RouteClass::General);
            live.routes.replace_own_class(RouteClass::General);
            continue;
        }
        live.track(RouteClass::General, Instant::now());
        // Only a heal's republish is paced (O3): a route Veilid replaced, a
        // renew or the first blob goes at once, as before RH1r-c.
        let now = Instant::now();
        let mut at = if heal { live.publish_at(now) } else { now };
        loop {
            // Biased, the newer blob first: a blob offered at the instant
            // the publish falls due supersedes the one about to be written.
            tokio::select! {
                biased;
                changed = offers.changed() => {
                    if changed.is_err() { return }
                    // A newer blob supersedes this one; keep the heal mark.
                    if heal { live.heal_pending.lock().insert(RouteClass::General); }
                    offers.mark_changed();
                    break;
                }
                () = tokio::time::sleep_until(at) => {}
            }
            match crate::broadcast::node::publish_personal_route(&session, &shared, &blob).await {
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
        self.classes
            .lock()
            .get(&RouteClass::General)
            .map_or(now, |s| s.rules.publish_at(now))
    }

    /// A publish went out at `now`.
    fn published(&self, now: Instant) {
        if let Some(state) = self.classes.lock().get_mut(&RouteClass::General) {
            state.rules.published(now);
        }
    }
}
