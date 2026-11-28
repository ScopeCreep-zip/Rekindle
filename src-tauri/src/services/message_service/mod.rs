//! What is left of the desktop's 1:1 message service once friends and
//! direct messages moved into the Host (plan E2.1): the unported payload
//! families' handlers (`services::veilid::inbound_unported` calls them) and
//! our own profile push for the relay pool (E5.2).

mod call_signaling;
mod dm_dispatch;
mod profile_push;

pub(crate) use call_signaling::handle_call_signaling_payload;
pub(crate) use dm_dispatch::handle_dm_payload;
pub use profile_push::push_profile_update;
