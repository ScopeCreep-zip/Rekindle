//! Chat error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChatError {
    #[error("transport: {0}")]
    Transport(#[from] rekindle_transport::TransportError),

    #[error("storage: {0}")]
    Storage(#[from] rekindle_storage::StorageError),

    #[error("ratchet: {0}")]
    Ratchet(#[from] rekindle_ratchet::RatchetError),

    #[error("not friends with {peer_key}")]
    NotFriends { peer_key: String },

    #[error("already friends with {peer_key}")]
    AlreadyFriends { peer_key: String },

    #[error("friend request not found for {peer_key}")]
    RequestNotFound { peer_key: String },

    #[error("friend inbox not available for target")]
    InboxNotAvailable,

    #[error("no outbound DM log for {peer_key}")]
    NoOutboundLog { peer_key: String },

    #[error("no inbound DM log for {peer_key}")]
    NoInboundLog { peer_key: String },

    #[error("message too large ({len} bytes, max {max})")]
    MessageTooLarge { len: usize, max: usize },

    #[error("not a member of community {community}")]
    NotMember { community: String },

    #[error("community not found: {community}")]
    CommunityNotFound { community: String },

    #[error("channel not found: {channel} in {community}")]
    ChannelNotFound { community: String, channel: String },

    #[error("MEK not cached for {community}/{channel}")]
    MekNotCached { community: String, channel: String },

    #[error("insufficient permissions: {action}")]
    InsufficientPermissions { action: String },

    #[error("identity not initialized")]
    NotInitialized,

    #[error("identity already initialized")]
    AlreadyInitialized,

    #[error("no Signal session for {peer_key}")]
    NoSession { peer_key: String },

    #[error("session lock timeout")]
    SessionLockTimeout,

    #[error("serialization: {0}")]
    Serialization(String),

    #[error("deserialization: {0}")]
    Deserialization(String),

    #[error("signing key not loaded")]
    SigningKeyNotLoaded,

    #[error("internal: {0}")]
    Internal(String),
}
