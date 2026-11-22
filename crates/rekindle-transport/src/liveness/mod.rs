//! Route liveness on the transport (plan RH1r-c; live finding 75;
//! production research batch 12): the rules of
//! [`rekindle_route::liveness`], driven over the ports.
//!
//! - **Owner** ([`owner`]): a round-trip self-test of our own routes
//!   (General, Media in use, Sending) through [`AppMessagePort::call_own`], Veilid's own loopback
//!   (O1, O6); a silence trigger (O2); on death, a replacement, tested and
//!   then republished under the DHT's next seq, the old route kept until
//!   then (O3, O4); an immediate test on a reconnect (O5).
//! - **Sender** ([`peer`]): a peer's route is suspected from missing acks,
//!   re-read from their profile at a bounded rate and probed, and the peer
//!   shown unreachable after six rounds (S1–S5).
//! - **Re-resolve** ([`resolve`]): one profile read in flight per peer
//!   with no usable route, a 5 s backoff after a failed one (CR2).
//!
//! The probe is `'L'` ‖ 16 random bytes (`rekindle_codec::ingress`
//! `LIVENESS_TAG`). Our dispatch echoes only a nonce we issued; any other
//! is answered with a NAK, which tells a prober only that its call arrived.

pub mod owner;
pub mod peer;
pub mod resolve;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_codec::ingress::{LIVENESS_PROBE_LEN, LIVENESS_TAG};
use rekindle_records::port::{AppMessagePort, RouteClass, RoutePort};
use rekindle_route::liveness::{InboundLiveness, PeerLiveness, SELF_TEST_JITTER_MS};
use rekindle_utils::{Clock, RandomSource};
use tokio::sync::Notify;
use tokio::time::Instant;

/// The classes we self-test: General always, Media while a call carries
/// media over it, and Sending always (plan RT1: on Linux both Sending deaths
/// in 21 app-hours came 200/331 s after allocation, never tested). Sending
/// is a private route we allocated like the others, never published: the
/// safety route our 1:1 calls and statements are pinned to and their
/// answers' reply route. Veilid's own loopback test of an allocated route is
/// the same call for any of them, the route as its own safety route
/// (veilid-core 0.5.8 `ping_validator.rs:597-612`, accepted as a loopback
/// at `route_compile.rs:420-434` whatever its hop count), and Veilid tests an
/// unpublished route on its own once only while it has never passed
/// (`private_route_management.rs:116-123`), dropping its failures while
/// Offline (`ping_validator.rs:666-690`). One 17-byte probe per 40-53.3 s
/// per class when healthy; after a failure, retests 10-20 s apart, three
/// failures and the route is replaced.
///
/// General and Sending are tested each period whatever arrives over them
/// (plan LF6, lead decision 39; [`InboundLiveness::ungated`]); Media keeps
/// the idle gate, being tested only while a call uses it. A peer's suspect
/// round probes our General route (S2), so a foreign probe that reaches us
/// is that peer's report that our route failed it: it brings General's
/// test forward ([`Liveness::route_reported`]).
const TESTED: [RouteClass; 3] = [RouteClass::General, RouteClass::Media, RouteClass::Sending];

/// The answer to an inbound call that may be a liveness probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeAnswer {
    /// Not a probe: dispatch it as any call.
    NotProbe,
    /// Our own probe: echo it.
    Echo(Vec<u8>),
    /// Another's probe: NAK it.
    Refuse,
}

/// One inbound class's state.
struct ClassState {
    /// The live route's id these rules are for.
    id: Option<String>,
    rules: InboundLiveness,
    /// When inbound last arrived over it (any frame).
    heard: Option<Instant>,
}

/// Route liveness, shared by the dispatch loop, the senders and the owner
/// task of one transport node.
pub struct Liveness {
    rng: Arc<dyn RandomSource>,
    /// The wall clock that shows a resume from a suspend (plan LF7).
    clock: Arc<dyn Clock>,
    /// General's next self-test runs from outside, as a peer's call
    /// (`OwnPath::Outside`): set by a resume or a peer's report (plan LF7).
    outside_next: AtomicBool,
    routes: Arc<dyn RoutePort>,
    messages: Arc<dyn AppMessagePort>,
    classes: Mutex<HashMap<RouteClass, ClassState>>,
    /// Nonces of our self-tests in flight.
    outstanding: Mutex<HashSet<[u8; 16]>>,
    peers: Mutex<HashMap<String, PeerLiveness>>,
    /// A reconnect or readiness change (O5).
    network_changed: Notify,
    /// Classes whose replacement must pass a self-test before it is
    /// published (O3).
    heal_pending: Mutex<HashSet<RouteClass>>,
    /// Whether this node answers its own probes (it dispatches its own
    /// events); an outbound-only node never self-tests.
    self_tests: bool,
    /// One re-resolve of a peer's route in flight at a time (CR2).
    resolves: resolve::ResolveFlights,
}

impl Liveness {
    /// Liveness over `routes` and `messages`, its jitter drawn from `rng`,
    /// a resume read from `clock`.
    #[must_use]
    pub fn new(
        rng: Arc<dyn RandomSource>,
        clock: Arc<dyn Clock>,
        routes: Arc<dyn RoutePort>,
        messages: Arc<dyn AppMessagePort>,
        self_tests: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            rng,
            clock,
            outside_next: AtomicBool::new(false),
            routes,
            messages,
            classes: Mutex::new(HashMap::new()),
            outstanding: Mutex::new(HashSet::new()),
            peers: Mutex::new(HashMap::new()),
            network_changed: Notify::new(),
            heal_pending: Mutex::new(HashSet::new()),
            self_tests,
            resolves: resolve::ResolveFlights::default(),
        })
    }

    /// Whether this node self-tests its routes.
    #[must_use]
    pub fn self_tests(&self) -> bool {
        self.self_tests
    }

    /// A jitter in `[0, bound_ms)` ms from the RNG port.
    fn jitter(&self, bound_ms: u64) -> Duration {
        Duration::from_millis(self.rng.below(bound_ms))
    }

    /// Inbound arrived over our route `route` (the event's id text).
    pub fn heard_over(&self, route: Option<&str>) {
        let Some(route) = route else {
            return;
        };
        let now = Instant::now();
        let mut classes = self.classes.lock();
        for state in classes.values_mut() {
            if state.id.as_deref() == Some(route) {
                state.rules.heard(now);
                state.heard = Some(now);
            }
        }
    }

    /// What to answer an inbound call `raw` that may be a liveness probe:
    /// our own nonce comes back as it went; any other probe is refused.
    #[must_use]
    pub fn answer_probe(&self, raw: &[u8]) -> ProbeAnswer {
        if raw.len() != LIVENESS_PROBE_LEN || raw.first() != Some(&LIVENESS_TAG) {
            return ProbeAnswer::NotProbe;
        }
        let mut nonce = [0u8; 16];
        nonce.copy_from_slice(&raw[1..]);
        if self.outstanding.lock().contains(&nonce) {
            ProbeAnswer::Echo(raw.to_vec())
        } else {
            ProbeAnswer::Refuse
        }
    }

    /// A fresh probe datagram, its nonce noted as ours until `forget`.
    fn probe(&self) -> ([u8; 16], Vec<u8>) {
        let mut nonce = [0u8; 16];
        self.rng.fill(&mut nonce);
        let mut datagram = Vec::with_capacity(LIVENESS_PROBE_LEN);
        datagram.push(LIVENESS_TAG);
        datagram.extend_from_slice(&nonce);
        (nonce, datagram)
    }

    /// The network reconnected or became ready again (O5).
    pub fn network_changed(&self) {
        let now = Instant::now();
        for state in self.classes.lock().values_mut() {
            state.rules.network_changed(now);
        }
        self.network_changed.notify_one();
    }

    /// A peer reported our route failed it: its suspect round's probe
    /// reached us (S2). The General route's next self-test falls due at once
    /// (at most once per `REPORT_TEST_GAP`); the test, not the report,
    /// decides whether the route is dead. Returns whether a test was
    /// brought forward.
    pub fn route_reported(&self) -> bool {
        let brought = self
            .classes
            .lock()
            .get_mut(&RouteClass::General)
            .is_some_and(|s| s.rules.reported(Instant::now()));
        if brought {
            tracing::info!("a peer's probe reported our route; self-testing it now from outside");
            self.outside_next.store(true, Ordering::Relaxed);
            self.network_changed.notify_one();
        }
        brought
    }

    /// Our process resumed from a suspend (plan LF7): every route is tested
    /// at once and one failure is death (as O5), and General's test runs
    /// from outside. Our loopback cannot see what a pause breaks for peers:
    /// it re-primes the hop next to us right before it measures it, and the
    /// entry hop turns it around before the blob's entry is ever resolved
    /// (`OwnPath`). A failed test replaces and republishes the route.
    pub fn resumed(&self) {
        tracing::info!("resumed from a suspend; testing our routes from outside");
        self.outside_next.store(true, Ordering::Relaxed);
        self.network_changed();
    }

    /// The wall clock now, ms.
    fn wall_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Whether any peer should be answering us now (O2): a send to them
    /// awaits its answer.
    fn expecting(&self) -> bool {
        self.peers.lock().values().any(PeerLiveness::awaiting)
    }

    /// Follow our live route of `class`: a new one starts its rules over.
    /// Returns whether it is live.
    pub(crate) fn track(&self, class: RouteClass, now: Instant) -> bool {
        let id = self.routes.own_id(class);
        let mut classes = self.classes.lock();
        let state = classes.entry(class).or_insert_with(|| ClassState {
            id: None,
            rules: if class == RouteClass::Media {
                InboundLiveness::new(now, Duration::ZERO)
            } else {
                InboundLiveness::ungated(now, Duration::ZERO)
            },
            heard: None,
        });
        if state.id != id {
            // The jitter is drawn only when a route comes or goes.
            state
                .rules
                .route_replaced(now, self.jitter(SELF_TEST_JITTER_MS));
            state.id.clone_from(&id);
            state.heard = None;
        }
        id.is_some()
    }
}
