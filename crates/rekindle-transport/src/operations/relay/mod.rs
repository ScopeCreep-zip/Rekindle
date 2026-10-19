//! Alice's side of Strand Relay, pulled forward from E5.2 because the 1:1
//! send path calls it (plan E2.1): sending through a friend's relay when
//! the direct route fails (§13.3), and asking friends for a peer's status
//! and route (§13.5). Their wire is unchanged (V18); the relay's receive
//! side stays for E5.2.

mod probe;
mod send;

pub(crate) use probe::on_status_response;
pub use probe::probe_friends_for_status;
pub use send::send_via_relay;
