//! Typed DHT record helpers over the session's record pool.
//!
//! Every record operation goes through the session's `RecordPort` (plans
//! C7, C9.1). Our own profile, mailbox and friend list are
//! `rekindle_protocol::dht::{profile, mailbox, friends}`; channel messages
//! are [`channel_smpl`].

pub mod channel_smpl;
