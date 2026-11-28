//! Strand Relay Network (architecture §13).
//!
//! Friend-to-friend relay infrastructure: Carol volunteers as a relay for
//! Bob; Alice — when she cannot reach Bob directly — sends through Carol
//! who forwards an opaque encrypted blob. Implements the three roles:
//!
//! - [`offer`] — Carol's side: create a dedicated relay route, send
//!   `RelayOffer` (0x0130) to Bob by `app_call`.
//! - [`pool`]  — Bob's side: persist offers received from friends, expose
//!   the relay pool for publication on his profile DHT record.
//! - [`forward`] — Carol's side again: receive a `RelayEnvelope`
//!   (0x0003), check she volunteered for its target, forward the inner
//!   `'D'` datagram to that friend's current route.
//! - [`presence`] — Carol's side: answer a `StatusRequest` (0x0140).
//!
//! Alice's side (sending through a relay when the direct route fails)
//! returns at E5.5, whose pool entries name the relay's identity
//! (`rekindle_transport::operations::dm::send`). These stay here, reached
//! through `Routed::Unported`, until E5.2 (plan E2.3).

pub mod forward;
pub mod offer;
pub mod pool;
pub mod presence;

pub use forward::handle_relay_envelope;
pub use offer::{list_volunteered_for, revoke_relay, volunteer_relay};
pub use pool::{add_received_offer, list_received_offers, remove_received_offer};
pub use presence::respond_to_status_request;
