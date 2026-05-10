//! Inbound event processing — VeilidUpdate dispatch to TransportCallback.
//!
//! The dispatch loop receives VeilidUpdate events from the Veilid node's
//! update channel and forwards raw bytes to the TransportCallback. Transport
//! performs two optimizations before forwarding:
//!
//! 1. **Gossip dedup (TypeId 0x0A):** BLAKE3 content hash suppresses duplicate
//!    gossip envelopes arriving from multiple mesh peers.
//! 2. **Value routing:** ValueChange events include the record key and changed
//!    subkeys so chat can route to the correct handler without re-reading.
//!
//! All payload parsing, signature verification, decryption, and semantic
//! routing happen in rekindle-chat's EventRouter after raw bytes arrive.

pub mod dispatch;
