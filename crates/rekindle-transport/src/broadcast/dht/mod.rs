//! Typed DHT record helpers over the session's record pool.
//!
//! Every record operation goes through `rekindle_protocol`'s `RecordPool`
//! (plan C7). Our own profile, mailbox and friend list are
//! `rekindle_protocol::dht::{profile, mailbox, friends}`; channel messages
//! are [`channel_smpl`]; the append-only log is [`channel_log`].

pub mod channel_log;
pub mod channel_smpl;
