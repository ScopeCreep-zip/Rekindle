//! One-shot CLI command handlers.
//!
//! Each module constructs an `IpcRequest`, sends it via `DaemonClient`,
//! deserializes the response, and renders output in the appropriate format.
//! No business logic — that lives in `rekindle-chat`.

pub mod identity;
pub mod community;
pub mod channel;
pub mod friends;
pub mod dm;
pub mod governance;
pub mod keys;
pub mod network;
pub mod presence;
pub mod voice;
pub mod social;
pub mod system;
pub mod node_daemon;
pub mod patch;
pub mod search;
