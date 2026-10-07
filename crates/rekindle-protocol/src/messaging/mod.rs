//! Sending and receiving 1:1 envelopes over Veilid. The envelope types
//! live in `rekindle_codec::message` (plan C8).

pub mod receiver;
pub mod replay;
pub mod sender;

pub use receiver::process_incoming;
