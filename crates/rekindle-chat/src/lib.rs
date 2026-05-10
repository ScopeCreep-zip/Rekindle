//! Chat application logic for the Rekindle encrypted messaging platform.
//!
//! Composes `rekindle-ratchet` (encrypt/decrypt), `rekindle-storage`
//! (persist), and `rekindle-transport` (send/receive via trait) into
//! complete user-facing messaging operations.
//!
//! This crate contains ALL business logic. `rekindle-node` is a thin
//! daemon shell that forwards IPC requests to `ChatService` methods.
//! `rekindle-transport-veilid` sends bytes. This crate decides what
//! those bytes mean.

pub mod error;
mod time;
pub mod io;
pub mod service;
pub mod crypto;
pub mod messaging;
pub mod friendship;
pub mod community;
pub mod identity;
pub mod events;
pub mod presence;
pub mod voice;

pub use error::ChatError;
pub use io::PlatformIO;
pub use service::ChatService;
