//! App messages and calls to peers' private routes.

use super::{PeerTarget, PortError, RouteClass};

use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

/// Which way a round trip through our own route travels (plan LF7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnPath {
    /// The route as its own safety route, Veilid's loopback
    /// (veilid-core 0.5.8 `route_compile.rs:420-435`): we send to its last
    /// hop ourselves and the entry hop turns it around without transiting
    /// the network (`rpc_processor/rpc_route.rs:329-365`), so it re-primes
    /// the hop next to us (a fresh flow, our current peer info,
    /// `rpc_processor/mod.rs:895-897`) right before it measures it.
    Loopback,
    /// From outside, as a peer's call arrives: our route's blob over our
    /// `Sending` route's warm standby as the safety route, apart from the
    /// route (the shape of Veilid's remote-route test,
    /// `route_test.rs:345-388`), so it enters by the blob's entry hop and
    /// reaches us through the hop next to us unprompted; the answer returns
    /// over the standby (`rpc_processor/destination.rs:498-516`), never over
    /// the live `Sending` route our pinned sends ride, so a failing test
    /// charges nothing to it (plans LF9, LF9c).
    Outside,
}

/// An inbound app call awaiting its reply, by the id the network gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppCallId(pub u64);

/// `Err(MessageTooLarge)` for a payload over Veilid's message cap
/// (`rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN`).
///
/// # Errors
/// The payload is too large.
pub fn check_message_len(message: &[u8]) -> Result<(), PortError> {
    if message.len() > MAX_APP_MESSAGE_LEN {
        return Err(PortError::MessageTooLarge {
            len: message.len(),
            max: MAX_APP_MESSAGE_LEN,
        });
    }
    Ok(())
}

/// Messages and calls over private routes. `profile` is the safety-route
/// hop count and sequencing the class asks for
/// (`rekindle_types::config::SafetyProfile`). A payload over the message
/// cap is refused before it is sent ([`check_message_len`]).
#[async_trait::async_trait]
pub trait AppMessagePort: Send + Sync {
    /// Send `message` to `target`, fire and forget, with our live
    /// `Sending` route as the safety route (Veilid's
    /// `SafetySpec.preferred_route`), the route our 1:1 calls are pinned to
    /// ([`call_pinned`](Self::call_pinned)). A statement over an untested
    /// pool route is lost silently when that route fails: Veilid counts and
    /// steers on lost questions only (0.5.8 `route_stats.rs:144-147`,
    /// `route_select.rs:321-410`). A statement names no reply route, so the
    /// recipient learns nothing of ours (`rpc_processor/mod.rs:1060-1063`).
    ///
    /// # Errors
    /// `Routing` while our `Sending` route is not live: there is no pool
    /// route fallback; otherwise the send failed, and `RouteUnusable` asks
    /// the caller to invalidate.
    async fn send(
        &self,
        target: &PeerTarget,
        profile: &rekindle_types::config::SafetyProfile,
        message: Vec<u8>,
    ) -> Result<(), PortError>;

    /// Send `message` to `target` and await the reply, over a safety and
    /// reply route Veilid picks from its pool: a call hands its safety
    /// route to the callee as the reply route, so only a 1:1 call is pinned
    /// ([`call_pinned`](Self::call_pinned)); a community call pinned to our
    /// `Sending` route would show members the blob our friends see (threat
    /// model L2).
    ///
    /// # Errors
    /// The call failed or timed out; `RouteUnusable` as for `send`.
    async fn call(
        &self,
        target: &PeerTarget,
        profile: &rekindle_types::config::SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError>;

    /// [`call`](Self::call) with our live route of `class` as the safety
    /// route, so the answer comes back over it too (veilid-core
    /// 0.5.8 `rpc_processor/destination.rs:467-508`).
    ///
    /// # Errors
    /// `Routing` while our route of `class` is not live: there is no pool
    /// route fallback; otherwise as for `call`.
    async fn call_pinned(
        &self,
        target: &PeerTarget,
        profile: &rekindle_types::config::SafetyProfile,
        class: RouteClass,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError>;

    /// A round trip through our own live route of `class` (plan RH1r-c
    /// O1, O6; LF7 `path`): with [`OwnPath::Loopback`] `message` is called
    /// to our route with the same route as the safety route, so it leaves
    /// through the route's hops and comes back in through them, and our own
    /// dispatch answers it; with [`OwnPath::Outside`] it enters as a peer's. Veilid's own
    /// loopback test takes this path (`routing_table/tasks/ping_validator.rs`
    /// `:498-514`): the private route is also the safety route
    /// (`route_spec_store/route_compile.rs:385-400`), the hop turns it
    /// around (`rpc_processor/rpc_route.rs:292-320`) and the answer comes
    /// back over the route (`rpc_processor/destination.rs:467-480`).
    ///
    /// # Errors
    /// `Routing` while our route of `class` is not live; otherwise as for
    /// `call` (a timeout is a failed test).
    async fn call_own(
        &self,
        class: RouteClass,
        path: OwnPath,
        profile: &rekindle_types::config::SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError>;
    /// Answer the inbound call `call_id`.
    ///
    /// # Errors
    /// The reply could not be sent.
    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError>;
}
