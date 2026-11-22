//! Route liveness on the transport (plan RH1r-c; live finding 75;
//! production research batch 12): the rules of
//! [`rekindle_route::liveness`], driven over the ports.
//!
//! - **Owner** ([`owner`]): our routes are tested by Veilid itself (a
//!   loopback keepalive every 10 s on each allocated route, veilid-core
//!   0.5.8 `private_route_management.rs:7`) and a death arrives as
//!   `RouteChange.dead_routes`, which `OwnRoutes::on_dead` handles. A
//!   General route dead only from outside is rotated on evidence (plan
//!   RV1): inbound silence while a peer should be answering, or a peer's
//!   `'L'` report; the replacement is republished under the DHT's next seq
//!   and the old route released once that lands (O3, O4).
//! - **Sender** ([`peer`]): a peer's route is suspected from missing acks,
//!   re-read from their profile at a bounded rate and probed, and the peer
//!   shown unreachable after six rounds (S1–S5).
//! - **Re-resolve** ([`resolve`]): one profile read in flight per peer
//!   with no usable route, a 5 s backoff after a failed one (CR2).
//!
//! The probe is `'L'` ‖ 16 random bytes (`rekindle_codec::ingress`
//! `LIVENESS_TAG`). Our dispatch answers every probe with a NAK, which tells
//! a prober only that its call arrived, and takes it as that peer's report.

pub mod owner;
pub mod peer;
pub mod resolve;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_codec::ingress::{LIVENESS_PROBE_LEN, LIVENESS_TAG};
use rekindle_records::port::{RouteClass, RoutePort};
use rekindle_route::liveness::{InboundLiveness, PeerLiveness};
use rekindle_utils::RandomSource;
use tokio::time::Instant;

/// The answer to an inbound call that may be a liveness probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeAnswer {
    /// Not a probe: dispatch it as any call.
    NotProbe,
    /// A peer's probe, its report that our route failed it: NAK it.
    Refuse,
}

/// Our General route's state.
struct GeneralState {
    /// The live route's id these rules are for.
    id: Option<String>,
    rules: InboundLiveness,
}

/// Route liveness, shared by the dispatch loop, the senders and the owner
/// task of one transport node.
pub struct Liveness {
    rng: Arc<dyn RandomSource>,
    routes: Arc<dyn RoutePort>,
    general: Mutex<Option<GeneralState>>,
    peers: Mutex<HashMap<String, PeerLiveness>>,
    /// A rotation's replacement waits for its paced republish (O3).
    rotated: AtomicBool,
    /// Whether this node sees its inbound (it, or its host, dispatches its
    /// events), so it can act on inbound evidence; an outbound-only node
    /// with no dispatch never rotates.
    sees_inbound: bool,
    /// One re-resolve of a peer's route in flight at a time (CR2).
    resolves: resolve::ResolveFlights,
}

impl Liveness {
    /// Liveness over `routes`, its jitter drawn from `rng`.
    #[must_use]
    pub fn new(
        rng: Arc<dyn RandomSource>,
        routes: Arc<dyn RoutePort>,
        sees_inbound: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            rng,
            routes,
            general: Mutex::new(None),
            peers: Mutex::new(HashMap::new()),
            rotated: AtomicBool::new(false),
            sees_inbound,
            resolves: resolve::ResolveFlights::default(),
        })
    }

    /// Whether this node acts on inbound evidence about its routes.
    #[must_use]
    pub fn sees_inbound(&self) -> bool {
        self.sees_inbound
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
        if let Some(state) = self.general.lock().as_mut() {
            if state.id.as_deref() == Some(route) {
                state.rules.heard(Instant::now());
            }
        }
    }

    /// What to answer an inbound call `raw` that may be a liveness probe:
    /// every probe is a peer's and is refused.
    #[must_use]
    pub fn answer_probe(&self, raw: &[u8]) -> ProbeAnswer {
        if raw.len() == LIVENESS_PROBE_LEN && raw.first() == Some(&LIVENESS_TAG) {
            ProbeAnswer::Refuse
        } else {
            ProbeAnswer::NotProbe
        }
    }

    /// A fresh probe datagram for a peer's route.
    fn probe(&self) -> Vec<u8> {
        let mut nonce = [0u8; 16];
        self.rng.fill(&mut nonce);
        let mut datagram = Vec::with_capacity(LIVENESS_PROBE_LEN);
        datagram.push(LIVENESS_TAG);
        datagram.extend_from_slice(&nonce);
        datagram
    }

    /// A hint from friend `peer` arrived over our route (plan RV1, the
    /// third evidence source): their chain is not ahead of it.
    pub fn hinted(&self, peer: &str) {
        if let Some(state) = self.general.lock().as_mut() {
            state.rules.hinted(peer, Instant::now());
        }
    }

    /// Friend `peer`'s DM chain showed a new entry (its watch fired): the
    /// owner task rotates our General route if the hint it should have
    /// carried does not arrive ([`rekindle_route::liveness::HINT_WINDOW`]).
    pub fn chain_changed(&self, peer: &str) {
        if let Some(state) = self.general.lock().as_mut() {
            state.rules.chain_changed(peer, Instant::now());
        }
    }

    /// A peer reported our route failed it: its suspect round's probe
    /// reached us (S2), and the reporter is online. Our General route is
    /// rotated unless it is younger than one re-fetch gap
    /// ([`rekindle_route::liveness::REPORT_ROTATE_AGE`]: the report is about
    /// the route it replaced). Returns whether it is rotated.
    pub fn route_reported(&self) -> bool {
        let rotate = self.sees_inbound
            && self
                .general
                .lock()
                .as_ref()
                .is_some_and(|s| s.id.is_some() && s.rules.reported(Instant::now()));
        if rotate {
            self.rotate("a peer reported it");
        }
        rotate
    }

    /// Rotate our General route on evidence that it is dead from outside
    /// (plan RV1): `OwnRoutes::replace` allocates a new one (Veilid tests
    /// it at allocation, 0.5.8 `veilid_api/api.rs:831`) and keeps the old
    /// one until the publisher's write of the new blob lands (O4).
    fn rotate(&self, why: &str) {
        tracing::warn!(why, "rotating our General route on evidence");
        self.rotated.store(true, Ordering::SeqCst);
        self.routes.replace_own_class(RouteClass::General);
    }

    /// Whether any peer should be answering us now (O2): a send to an
    /// online peer awaits its answer.
    fn expecting(&self) -> bool {
        self.peers.lock().values().any(PeerLiveness::awaiting)
    }

    /// Follow our live General route: a new one starts its rules over.
    /// Returns whether it is live.
    pub(crate) fn track(&self, now: Instant) -> bool {
        let id = self.routes.own_id(RouteClass::General);
        let mut general = self.general.lock();
        let state = general.get_or_insert_with(|| GeneralState {
            id: None,
            rules: InboundLiveness::new(now),
        });
        if state.id != id {
            state.rules.route_replaced(now);
            state.id.clone_from(&id);
        }
        id.is_some()
    }
}
