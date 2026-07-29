pub mod state;
pub mod handshake;
pub mod driver;
pub mod rotation;

/// Role of this peer in the session — determines key direction mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    /// We are the dialler (initiator). Outbound = d2l, inbound = l2d.
    Dialler,
    /// We are the listener (responder). Outbound = l2d, inbound = d2l.
    Listener,
}
