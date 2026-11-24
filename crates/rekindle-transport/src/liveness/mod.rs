//! Route liveness on the transport (plan RH1r-c; live finding 75;
//! production research batch 12): the rules of
//! [`rekindle_route::liveness`], driven over the ports.
//!
//! - **Owner** ([`owner`]): our routes are tested by Veilid itself (a
//!   loopback keepalive every 10 s on each allocated route, veilid-core
//!   0.5.8 `private_route_management.rs:7`) and a death arrives as
//!   `RouteChange.dead_routes`, which `OwnRoutes::on_dead` handles. A
//!   General route dead only from outside is rotated on evidence (plan
//!   RV1, RV2): inbound silence while a peer should be answering, a
//!   friend's DM chain ahead of its hints, or a friend re-sending a call we
//!   receipted over it; the replacement is republished under the DHT's next
//!   seq and the old route released once that lands (O3, O4).
//! - **Sender** ([`peer`]): a peer's route is suspected from missing acks,
//!   re-read from their profile at a bounded rate and probed, and the peer
//!   shown unreachable after six rounds (S1–S5).
//! - **Re-resolve** ([`resolve`]): one profile read in flight per peer
//!   with no usable route, a 5 s backoff after a failed one (CR2).
//!
//! The probe is `'L'` ‖ 16 random bytes (`rekindle_codec::ingress`
//! `LIVENESS_TAG`). Our dispatch answers every probe with a NAK, which tells
//! a prober only that its call arrived. A probe is no evidence about our
//! route (plan RV2): it names no sender, and Veilid names none for a call
//! over a private route (0.5.8 `veilid_api/types/app_message_call.rs`,
//! `sender` is `None` then), so anyone who read our published blob could
//! send it and drain the route budget.

pub mod dead;
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
    /// A peer's probe of our route: NAK it, which proves only that it
    /// arrived; it counts as no evidence (plan RV2).
    Refuse,
}

/// What to answer an inbound call `raw` that may be a liveness probe: every
/// probe is a peer's (RV1 removed our own self-test probes, so none is ours)
/// and is refused, unattributed and so never counted (plan RV2). It reads only the datagram, so it needs no node state.
#[must_use]
pub fn answer_probe(raw: &[u8]) -> ProbeAnswer {
    if raw.len() == LIVENESS_PROBE_LEN && raw.first() == Some(&LIVENESS_TAG) {
        ProbeAnswer::Refuse
    } else {
        ProbeAnswer::NotProbe
    }
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
    /// Peers' route blobs known dead, never imported or called while
    /// cached (RV1g B1–B3, [`dead`]).
    dead: Mutex<rekindle_route::dead_routes::DeadRoutes>,
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
            dead: Mutex::new(rekindle_route::dead_routes::DeadRoutes::default()),
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

    /// Whether `route` (an inbound event's route id) is our live General
    /// route.
    #[must_use]
    pub fn over_general(&self, route: Option<&str>) -> bool {
        route.is_some_and(|route| {
            self.general
                .lock()
                .as_ref()
                .is_some_and(|s| s.id.as_deref() == Some(route))
        })
    }

    /// Friend or open-handshake `peer` re-sent over our route `route` the
    /// call with digest `frame` that we had receipted (plan RV2, the fifth
    /// evidence source): counted only over our live General route; the
    /// owner task rotates it on [`rekindle_route::liveness::RECEIPT_LOST_REPEATS`]
    /// copies ([`owner::run_rotations`]).
    pub fn duplicate_call(&self, peer: &str, frame: [u8; 32], route: Option<&str>) {
        let Some(route) = route else {
            return;
        };
        if let Some(state) = self.general.lock().as_mut() {
            if state.id.as_deref() == Some(route) {
                state.rules.duplicate_call(peer, frame, Instant::now());
            }
        }
    }

    /// A call carrying the frame with digest `frame` arrived over our route
    /// `route` (plan RV2, evidence source 6): counted only over our live
    /// General route, it clears a durable copy of the same frame.
    pub fn call_seen(&self, frame: [u8; 32], route: Option<&str>) {
        let Some(route) = route else {
            return;
        };
        if let Some(state) = self.general.lock().as_mut() {
            if state.id.as_deref() == Some(route) {
                state.rules.call_seen(frame, Instant::now());
            }
        }
    }

    /// A durable first contact from `peer` with digest `frame` opened from
    /// one of our reply inboxes (plan RV2, evidence source 6): the owner
    /// task rotates our General route unless the call copy arrives over it
    /// within [`rekindle_route::liveness::HINT_WINDOW`].
    pub fn reply_landed(&self, peer: &str, frame: [u8; 32]) {
        if let Some(state) = self.general.lock().as_mut() {
            state.rules.reply_landed(peer, frame, Instant::now());
        }
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

#[cfg(test)]
#[path = "dead_tests.rs"]
mod dead_tests;
#[cfg(test)]
#[path = "receipt_tests.rs"]
mod receipt_tests;
