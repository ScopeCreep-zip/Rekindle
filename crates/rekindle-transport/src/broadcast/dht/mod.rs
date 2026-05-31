//! Typed DHT record operations.
//!
//! [`DhtStore`] is the entry point for all distributed hash table operations.
//! It wraps a Veilid `RoutingContext` and provides typed accessors for each
//! record type used by Rekindle.

pub mod account;
pub mod channel_log;
pub mod friend_list;
pub mod governance;
pub mod mailbox;
pub mod profile;
pub mod record;
pub mod registry;

use veilid_core::RoutingContext;

/// Entry point for all DHT record operations.
///
/// Obtained via [`TransportNode::dht()`](crate::node::TransportNode::dht).
/// Wraps a Veilid `RoutingContext` configured with the DHT safety profile.
pub struct DhtStore {
    rc: RoutingContext,
}

impl DhtStore {
    pub(crate) fn new(rc: RoutingContext) -> Self {
        Self { rc }
    }

    /// Access profile record operations.
    pub fn profile(&self) -> profile::ProfileOps<'_> {
        profile::ProfileOps::new(&self.rc)
    }

    /// Access friend list record operations.
    pub fn friend_list(&self) -> friend_list::FriendListOps<'_> {
        friend_list::FriendListOps::new(&self.rc)
    }

    /// Access mailbox record operations.
    pub fn mailbox(&self) -> mailbox::MailboxOps<'_> {
        mailbox::MailboxOps::new(&self.rc)
    }

    /// Access governance manifest record operations.
    pub fn governance(&self) -> governance::GovernanceOps<'_> {
        governance::GovernanceOps::new(&self.rc)
    }

    /// Access member registry record operations.
    pub fn registry(&self) -> registry::RegistryOps<'_> {
        registry::RegistryOps::new(&self.rc)
    }

    /// Access channel log record operations.
    pub fn channel_log(&self) -> channel_log::ChannelLogOps<'_> {
        channel_log::ChannelLogOps::new(&self.rc)
    }

    /// Access the underlying routing context.
    ///
    /// Used by [`QueryEngine`](crate::query::QueryEngine) for `DhtLog` operations
    /// that need direct routing context access.
    pub fn routing_context(&self) -> &RoutingContext {
        &self.rc
    }
}
