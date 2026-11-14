//! Route liveness on the transport (plan RH1r-c; live finding 75;
//! production research batch 12): the rules of
//! [`rekindle_route::liveness`], driven over the ports.
//!
//! - **Owner** ([`owner`]): a round-trip self-test of our published inbound
//!   routes through [`AppMessagePort::call_own`], Veilid's own loopback
//!   (O1, O6); a silence trigger (O2); on death, a replacement, tested and
//!   then republished under the DHT's next seq, the old route kept until
//!   then (O3, O4); an immediate test on a reconnect (O5).
//! - **Sender** ([`peer`]): a peer's route is suspected from missing acks,
//!   re-read from their profile at a bounded rate and probed, and the peer
//!   shown unreachable after six rounds (S1–S5).
//!
//! The probe is `'L'` ‖ 16 random bytes (`rekindle_codec::ingress`
//! `LIVENESS_TAG`). Our dispatch echoes only a nonce we issued; any other
//! is answered with a NAK, which tells a prober only that its call arrived.

pub mod owner;
pub mod peer;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_codec::ingress::{LIVENESS_PROBE_LEN, LIVENESS_TAG};
use rekindle_records::port::{AppMessagePort, RouteClass, RoutePort};
use rekindle_route::liveness::{InboundLiveness, PeerLiveness, SELF_TEST_JITTER_MS};
use rekindle_utils::RandomSource;
use tokio::sync::Notify;
use tokio::time::Instant;

/// The inbound classes we self-test: General always, Media while a call
/// carries media over it.
const TESTED: [RouteClass; 2] = [RouteClass::General, RouteClass::Media];

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
}

impl Liveness {
    /// Liveness over `routes` and `messages`, its jitter drawn from `rng`.
    #[must_use]
    pub fn new(
        rng: Arc<dyn RandomSource>,
        routes: Arc<dyn RoutePort>,
        messages: Arc<dyn AppMessagePort>,
        self_tests: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            rng,
            routes,
            messages,
            classes: Mutex::new(HashMap::new()),
            outstanding: Mutex::new(HashSet::new()),
            peers: Mutex::new(HashMap::new()),
            network_changed: Notify::new(),
            heal_pending: Mutex::new(HashSet::new()),
            self_tests,
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
            rules: InboundLiveness::new(now, Duration::ZERO),
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
