//! Inbound event processing — VeilidUpdate dispatch to mpsc channel.
//!
//! The dispatch loop receives VeilidUpdate events from the Veilid node's
//! update channel and sends typed `InboundEvent` to the mpsc channel.
//!
//! 1. **Gossip dedup (TypeId 0x0A):** BLAKE3 content hash suppresses duplicate
//!    gossip envelopes arriving from multiple mesh peers.
//! 2. **Bulk transfer (TypeId 0x30):** handled internally by the transport's
//!    TransferRegistry + DeliveryEngine. Never forwarded to the chat layer.
//! 3. **Value routing:** ValueChange events include the record key and changed
//!    subkeys so chat can route to the correct handler without re-reading.
//!
//! All payload parsing, signature verification, decryption, and semantic
//! routing happen in rekindle-chat's `run_inbound_loop` after events arrive.

pub mod dispatch;
