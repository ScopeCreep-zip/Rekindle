//! Streaming frame handlers — one function per HandoffKind variant.
//!
//! Each handler decodes the payload via the codec, validates against
//! HandoffState (arenas, sidechannel, pending fds), performs the arena
//! operation, and pushes outbound responses.

pub mod arena_write;
pub mod slot_release;
pub mod arena_setup;
pub mod arena_ack;
pub mod dmabuf_ref;
