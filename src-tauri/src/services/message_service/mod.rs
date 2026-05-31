//! `message_service` — incoming-message dispatch + friend-handshake
//! handlers + outgoing-message public API + DHT push helpers. Phase
//! 23.D split the original 2,255-LoC god module into focused
//! submodules; this file is now a re-export shim + module declarations
//! per Invariant 1 (≤500 LoC/file). All business logic lives in the
//! submodules listed below.

mod call_signaling;
mod dispatch;
mod friend_handlers;
mod outgoing;
mod profile_push;
mod session_reset;
mod transport;

pub use dispatch::{handle_incoming_message, try_handle_dm_invite_app_call};
pub(crate) use friend_handlers::delete_pending_messages_to_recipient;
pub(crate) use outgoing::build_and_queue_envelope;
pub use outgoing::{
    send_friend_accept, send_friend_reject, send_friend_request, send_message, send_to_peer_call,
    send_to_peer_encrypted, send_to_peer_raw, send_typing,
};
pub use profile_push::{push_friend_list_update, push_profile_update};
pub(crate) use transport::try_fetch_route_from_dht;
