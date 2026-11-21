//! One re-resolve of a peer's route in flight at a time (live finding CR2).
//!
//! While a peer has no usable route, every send re-reads their profile
//! inline (CR1); DM video alone is up to ~30 frames a second, so without
//! this each frame cost a DHT read. Now the first send leads: it reads the
//! profile, imports the route and sends. A send that arrives meanwhile
//! either waits for the leader's result (a durable kind: a call signal, a
//! handshake, a stream row) and then sends over the route the leader
//! cached, or fails at once (DM video and typing: a frame late is worse
//! than none, D24). A failed lead holds the peer back for
//! [`RESOLVE_BACKOFF`], so a peer with no route costs one read per 5 s,
//! not one per frame. A lead whose send future is dropped clears its
//! flight (the guard), so no peer is left waiting on a flight nobody runs.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::watch;
use tokio::time::Instant;

use super::Liveness;

/// How long a failed re-resolve holds a peer back from the next one.
pub const RESOLVE_BACKOFF: Duration = Duration::from_secs(5);

/// How long a durable send waits for another send's re-resolve.
pub const RESOLVE_WAIT: Duration = Duration::from_secs(10);

/// What a send's re-resolve came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    /// This send led the re-resolve; whether its send left this device.
    Led(bool),
    /// Another send's re-resolve landed and cached the route: send over it.
    Landed,
    /// No route: another re-resolve is in flight and this kind does not
    /// wait, the one waited for failed, was dropped or took over
    /// [`RESOLVE_WAIT`], or the peer is backing off after a failure.
    NotResolved,
}

/// One flight: its result, `None` until it lands.
type Flight = Arc<watch::Sender<Option<bool>>>;

/// One peer's re-resolve state.
#[derive(Default)]
struct PeerResolve {
    flight: Option<Flight>,
    backoff_until: Option<Instant>,
}

/// The re-resolves of every peer, one in flight per peer.
#[derive(Default)]
pub(super) struct ResolveFlights {
    peers: Mutex<HashMap<String, PeerResolve>>,
}

/// What a send does about a re-resolve.
enum Role {
    Lead(Flight),
    Follow(watch::Receiver<Option<bool>>),
    Refused,
}

/// Clears a flight its leader abandoned (the send future dropped).
struct FlightGuard<'a> {
    flights: &'a ResolveFlights,
    peer: &'a str,
    flight: Flight,
    landed: bool,
}

impl FlightGuard<'_> {
    /// The leader's attempt finished: a failure backs the peer off; the
    /// waiting sends learn the result.
    fn land(&mut self, sent: bool) {
        {
            let mut peers = self.flights.peers.lock();
            if sent {
                peers.remove(self.peer);
            } else if let Some(state) = peers.get_mut(self.peer) {
                if state
                    .flight
                    .as_ref()
                    .is_some_and(|f| Arc::ptr_eq(f, &self.flight))
                {
                    state.flight = None;
                }
                state.backoff_until = Some(Instant::now() + RESOLVE_BACKOFF);
            }
        }
        self.landed = true;
        self.flight.send_replace(Some(sent));
    }
}

impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        if self.landed {
            return;
        }
        // Abandoned: not a failure, so no backoff; the waiting sends see
        // the flight's sender go and give up.
        let mut peers = self.flights.peers.lock();
        let ours = peers
            .get(self.peer)
            .and_then(|s| s.flight.as_ref())
            .is_some_and(|f| Arc::ptr_eq(f, &self.flight));
        if ours {
            peers.remove(self.peer);
        }
    }
}

impl ResolveFlights {
    /// Re-resolve `peer`'s route through `attempt` (read the profile,
    /// import, send; whether it sent) unless one is in flight or the peer
    /// backs off; `wait` joins one in flight for up to [`RESOLVE_WAIT`].
    pub(super) async fn resolve<F, Fut>(&self, peer: &str, wait: bool, attempt: F) -> Resolve
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = bool>,
    {
        match self.role(peer, wait) {
            Role::Refused => Resolve::NotResolved,
            Role::Follow(rx) => follow(rx).await,
            Role::Lead(flight) => {
                let mut guard = FlightGuard {
                    flights: self,
                    peer,
                    flight,
                    landed: false,
                };
                let sent = attempt().await;
                guard.land(sent);
                Resolve::Led(sent)
            }
        }
    }

    fn role(&self, peer: &str, wait: bool) -> Role {
        let now = Instant::now();
        let mut peers = self.peers.lock();
        let state = peers.entry(peer.to_string()).or_default();
        if let Some(flight) = &state.flight {
            return if wait {
                Role::Follow(flight.subscribe())
            } else {
                Role::Refused
            };
        }
        if state.backoff_until.is_some_and(|until| now < until) {
            return Role::Refused;
        }
        let flight = Arc::new(watch::channel(None).0);
        state.flight = Some(Arc::clone(&flight));
        Role::Lead(flight)
    }
}

/// Wait for the flight's result, up to [`RESOLVE_WAIT`].
async fn follow(mut rx: watch::Receiver<Option<bool>>) -> Resolve {
    match tokio::time::timeout(RESOLVE_WAIT, rx.wait_for(Option::is_some)).await {
        Ok(Ok(result)) if *result == Some(true) => Resolve::Landed,
        _ => Resolve::NotResolved,
    }
}

impl Liveness {
    /// Re-resolve `peer`'s route once for every send that needs it now
    /// (live finding CR2): see [`ResolveFlights`]. `wait` is for a durable
    /// kind; DM video and typing pass `false` and fail fast.
    pub async fn resolve<F, Fut>(&self, peer: &str, wait: bool, attempt: F) -> Resolve
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = bool>,
    {
        self.resolves.resolve(peer, wait, attempt).await
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
