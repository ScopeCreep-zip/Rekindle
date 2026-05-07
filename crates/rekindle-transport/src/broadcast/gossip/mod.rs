//! Community gossip broadcast — organized by domain.
//!
//! Every outbound gossip message routes through this module. Each submodule
//! contains the public broadcast functions for a feature area. The `helpers`
//! module provides the shared sign-and-send machinery.
//!
//! All functions share the same pattern:
//! 1. Build typed `GossipPayload` or `ControlPayload`
//! 2. Call `helpers::build_sign_send()` or `helpers::control()`
//! 3. Return `BroadcastReport`

mod helpers;
mod message;
mod membership;
mod moderation;
mod crypto;
mod voice;
mod governance;
mod social;
mod system;
mod bootstrap;
mod ephemeral;

// Re-export everything for backward compatibility.
pub use helpers::{build_sign_send, send_direct, MeshMap};
pub use message::*;
pub use membership::*;
pub use moderation::*;
pub use crypto::*;
pub use voice::*;
pub use governance::*;
pub use social::*;
pub use system::*;
pub use bootstrap::*;
pub use ephemeral::*;
