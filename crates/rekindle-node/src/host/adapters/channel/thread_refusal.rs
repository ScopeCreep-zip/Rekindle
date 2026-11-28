//! A timed-out member's thread is refused at the Host's send path (plan
//! E3.1 F3c). Governance validates `ThreadCreated` at time 0, since a CRDT
//! merge reads no clock, so the timeout is checked here by the clock port;
//! a peer's such thread is held at receipt by the same rule
//! (`rekindle_channel::ingest::hold_timed_out_threads`, from
//! `CommunityStore::project_governance`).

use rekindle_channel::error::ChannelError;
use rekindle_channel::ingest::timed_out_until;
use rekindle_governance::permissions::{
    channel_permission_bit, channel_permission_name, CapabilityDenial,
};
use rekindle_governance::state::GovernanceState;
use rekindle_types::dto::community_capability::ChannelCapability;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey};

use super::HostChannel;
use crate::host::adapters::governance::capability_words::denial_words;

impl HostChannel {
    /// Check a thread control against the same rule used in the channel view.
    pub(super) fn require_thread_capability(
        &self,
        community: &str,
        channel: &str,
        capability: ChannelCapability,
    ) -> Result<(), ChannelError> {
        self.refuse_removed(community)?;
        let channel = ChannelId::from_hex(channel)
            .ok_or_else(|| ChannelError::InvalidId("Invalid parent channel.".into()))?;
        self.gov
            .require_own_permissions(
                community,
                Some(&channel),
                channel_permission_bit(capability),
            )
            .map_err(ChannelError::PermissionDenied)
    }
}

/// The governance writer checked creation permissions on its signing snapshot.
pub(super) fn thread_create_refused() -> ChannelError {
    let denial = CapabilityDenial::Missing {
        permission: channel_permission_name(ChannelCapability::CreatePublicThreads),
    };
    ChannelError::PermissionDenied(denial_words(denial, 0))
}

/// Refuse `entry` when it creates a thread and `writer` is timed out at
/// `now_secs`.
///
/// # Errors
/// `PermissionDenied`, naming when the timeout ends.
pub(super) fn refuse_timed_out_thread(
    entry: &GovernanceEntry,
    gov: &GovernanceState,
    writer: &PseudonymKey,
    now_secs: u64,
) -> Result<(), ChannelError> {
    if !matches!(entry, GovernanceEntry::ThreadCreated { .. }) {
        return Ok(());
    }
    match timed_out_until(gov, writer, now_secs) {
        Some(until_secs) => Err(ChannelError::PermissionDenied(denial_words(
            CapabilityDenial::TimedOut {
                until_secs,
                permission: channel_permission_name(ChannelCapability::CreatePublicThreads),
            },
            now_secs,
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "thread_refusal_tests.rs"]
mod tests;
