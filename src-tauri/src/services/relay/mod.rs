//! Strand Relay Network (architecture §13).
//!
//! Friend-to-friend relay infrastructure: Carol volunteers as a relay for
//! Bob; Alice — when she cannot reach Bob directly — sends through Carol
//! who forwards an opaque encrypted blob. Implements the three roles:
//!
//! - [`offer`] — Carol's side: create a dedicated relay route, send
//!   `RelayOffer` to Bob via `app_message`.
//! - [`pool`]  — Bob's side: persist offers received from friends, expose
//!   the relay pool for publication on his profile DHT record.
//! - [`forward`] — Carol's side again: receive a `RelayEnvelope`, look up
//!   the volunteer mapping `(route_id → friend)`, forward the inner
//!   payload to that friend's current route.
//! - [`send`] — Alice's side: when direct route fails, pick a relay entry
//!   from Bob's published pool, wrap the envelope, send through it.

pub mod forward;
pub mod health;
pub mod offer;
pub mod pool;
pub mod presence;
pub mod send;

pub use forward::handle_relay_envelope;
pub use offer::{list_volunteered_for, revoke_relay, volunteer_relay};
pub use pool::{add_received_offer, list_received_offers, remove_received_offer};
pub use presence::{handle_status_response, respond_to_status_request};
