//! App messages and calls to peers' private routes.

use super::{PeerTarget, PortError, RouteClass};

use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

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
    /// Send `message` to `target`, fire and forget.
    ///
    /// # Errors
    /// The send failed; `RouteUnusable` asks the caller to invalidate.
    async fn send(
        &self,
        target: &PeerTarget,
        profile: &rekindle_types::config::SafetyProfile,
        message: Vec<u8>,
    ) -> Result<(), PortError>;

    /// Send `message` to `target` and await the reply.
    ///
    /// # Errors
    /// The call failed or timed out; `RouteUnusable` as for `send`.
    async fn call(
        &self,
        target: &PeerTarget,
        profile: &rekindle_types::config::SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError>;

    /// [`send`](Self::send) with our live route of `class` as the safety
    /// route (Veilid's `SafetySpec.preferred_route`).
    ///
    /// # Errors
    /// `Routing` while our route of `class` is not live: there is no pool
    /// route fallback; otherwise as for `send`.
    async fn send_pinned(
        &self,
        target: &PeerTarget,
        profile: &rekindle_types::config::SafetyProfile,
        class: RouteClass,
        message: Vec<u8>,
    ) -> Result<(), PortError>;

    /// [`call`](Self::call) with our live route of `class` as the safety
    /// route, so the answer comes back over it too (veilid
    /// `rpc_processor/destination.rs:467-508`).
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

    /// Answer the inbound call `call_id`.
    ///
    /// # Errors
    /// The reply could not be sent.
    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError>;
}
