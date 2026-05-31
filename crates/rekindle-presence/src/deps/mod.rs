//! Phase 21 REDO — composite deps traits + shared DTOs.
//!
//! Decomposed per Invariant 1 (≤500 LoC per file): the friend-
//! presence and community-presence traits live in their own files
//! so each stays under the cap. Shared error/event types are in
//! this `mod.rs`; the trait re-exports here keep the existing
//! `crate::deps::*` import paths stable.

pub mod community;
pub mod friend;

pub use community::{
    CommunityPresenceDeps, DiscoveredMemberRow, OnlineMemberSnapshot, PresenceCredentials,
    SegmentDescriptor, SelfPresenceSnapshot,
};
pub use friend::{
    FriendPresenceDeps, FriendPresenceEvent, GameInfoSnapshot, SetFriendStatusOutcome,
};

/// Errors surfaced by the public entry points across both traits.
/// Lives at the module root so both `friend.rs` and `community.rs`
/// share one error vocabulary.
#[derive(Debug, thiserror::Error)]
pub enum PresenceError {
    #[error("no profile DHT key: node may not be ready")]
    MissingProfileKey,
    #[error("not attached to Veilid")]
    NotAttached,
    #[error("invalid DHT key: {0}")]
    InvalidDhtKey(String),
    #[error("DHT operation failed: {0}")]
    Dht(String),
}
