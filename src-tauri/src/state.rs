//! Central application-state facade. The 60+-field `AppState` struct
//! and its `Default` + small `impl` block live in `state/app_state.rs`;
//! per-domain helper types live in focused sibling submodules
//! (`automod`, `circuit`, `community`, `friend`, `gossip`, `runtime`).
//!
//! Consumer files continue to import via `crate::state::TypeName` —
//! every `pub use` below preserves the previous flat import surface.

use std::sync::Arc;

pub use rekindle_gossip::dedup::DedupCache;
pub use rekindle_gossip::mesh::fanout_degree as gossip_degree;

// Phase 23.B — helper types extracted into focused submodules.
pub mod app_state;
pub mod circuit;
pub mod community;
pub mod friend;
pub mod gossip;
pub mod runtime;

pub use app_state::AppState;
pub use rekindle_channel::{AutoModCompiledCache, CompiledAutoModRule};
pub use circuit::CircuitBreakerState;
pub use community::{
    display_role_name, CategoryInfo, ChannelInfo, ChannelType, CommunityRecords, CommunityState,
    EventRsvpEntry, MemberProfileSnapshot, RoleDefinition,
};
pub use friend::{FriendState, FriendshipState, GameInfoState, IdentityState, UserStatus};
pub use gossip::{GossipOverlay, OnlineMember};
pub use runtime::{
    DHTManagerHandle, GameDetectorHandle, NodeHandle, RoutingManagerHandle, SignalManagerHandle,
    VoiceEngineHandle,
};

/// Shared reference to `AppState`, used by both Tauri commands and background services.
pub type SharedState = Arc<AppState>;
