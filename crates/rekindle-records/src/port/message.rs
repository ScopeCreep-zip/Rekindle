//! App messages and calls to peers' private routes.

use super::{PeerTarget, PortError};

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
    /// Send `message` to `target`, fire and forget, over a safety route
    /// Veilid picks from its tested pool at the profile's hop count
    /// (veilid-core 0.5.8 `route_select.rs:119-124`; plan RV1). A statement
    /// names no reply route (`rpc_processor/mod.rs:1060-1063`).
    ///
    /// # Errors
    /// The send failed; `RouteUnusable` asks the caller to invalidate.
    async fn send(
        &self,
        target: &PeerTarget,
        profile: &rekindle_types::config::SafetyProfile,
        message: Vec<u8>,
    ) -> Result<(), PortError>;

    /// Send `message` to `target` and await the reply, over a safety route
    /// Veilid picks from its pool; the answer comes back over a pool route
    /// Veilid selects to match it (`rpc_processor/destination.rs:487-520`),
    /// so no route of ours is handed to the callee (threat model L2).
    ///
    /// # Errors
    /// The call failed or timed out; `RouteUnusable` as for `send`.
    async fn call(
        &self,
        target: &PeerTarget,
        profile: &rekindle_types::config::SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError>;

    /// Answer the inbound call `call_id`.
    ///
    /// # Errors
    /// The reply could not be sent.
    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError>;
}
