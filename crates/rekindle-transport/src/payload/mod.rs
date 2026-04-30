//! Payload type definitions for all data classes.
//!
//! Every byte that transmits over the network deserializes into one of
//! the types defined in these submodules. No `serde_json::Value` catch-alls.

pub mod dm;
pub mod gossip;
pub mod voice;
pub mod rpc;
pub mod dht_types;
