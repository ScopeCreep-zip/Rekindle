//! Alice's side of Strand Relay (plan E2.1): asking friends for a peer's
//! status and route (§13.5). Sending through a friend's relay (§13.3)
//! returns at E5.5, whose pool entries name the relay's identity (a relay
//! frame is addressed and signed to it); the relay's receive side stays
//! for E5.2.

mod probe;

pub(crate) use probe::on_status_response;
pub use probe::probe_friends_for_status;
