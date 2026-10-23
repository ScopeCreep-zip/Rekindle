//! The community audit log and notification default on both modes (plan
//! E3.1 F4).
//!
//! The audit log is read from the governance record by
//! `rekindle_governance_runtime::audit` (`VIEW_AUDIT_LOG`, every payload
//! verified, overflow followed) and projected here onto rows, newest first.
//! Every write the Host makes — a ban, a role grant, a channel edit — is a
//! governance entry, so it is an audit row the moment it is written; there
//! is no second log to keep in step. The notification default is the merged
//! `CommunityNotificationDefault`; setting it writes that entry.

use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::audit;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::dto::community::CommunityAuditEntry;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use super::audit_rows::governance_entry_to_audit_row;
use super::HostGovernance;

/// The most rows one page answers with.
pub const MAX_AUDIT_PAGE: u32 = 500;

/// One page of the log: every row older than `before` (when given), newest
/// first, at most `limit` (at least one, at most [`MAX_AUDIT_PAGE`]). Ties
/// are broken on every field so the order is total.
#[must_use]
pub fn audit_page(
    authored: Vec<(PseudonymKey, Vec<(u64, GovernanceEntry)>)>,
    before: Option<u64>,
    limit: u32,
) -> Vec<CommunityAuditEntry> {
    let mut rows: Vec<CommunityAuditEntry> = authored
        .into_iter()
        .flat_map(|(author, entries)| {
            let actor = hex::encode(author.0);
            entries
                .into_iter()
                .map(move |(lamport, entry)| governance_entry_to_audit_row(&actor, lamport, entry))
        })
        .filter(|row| before.is_none_or(|b| row.timestamp < b))
        .collect();
    rows.sort_by(|a, b| {
        b.timestamp
            .cmp(&a.timestamp)
            .then_with(|| a.actor_pseudonym.cmp(&b.actor_pseudonym))
            .then_with(|| a.action.cmp(&b.action))
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.details.cmp(&b.details))
    });
    rows.dedup();
    let page = usize::try_from(limit.clamp(1, MAX_AUDIT_PAGE)).unwrap_or(usize::MAX);
    rows.truncate(page);
    rows
}

/// The merged default notification level, if it is one readers accept.
#[must_use]
pub fn notification_default(state: &GovernanceState) -> Option<String> {
    state
        .notification_default
        .as_ref()
        .map(|d| d.level.clone())
        .filter(|level| audit::NOTIFICATION_LEVELS.contains(&level.as_str()))
}

impl HostGovernance {
    /// One page of the community's audit log.
    ///
    /// # Errors
    /// `VIEW_AUDIT_LOG` is missing, or the record could not be inspected.
    pub async fn audit_log(
        &self,
        community_id: &str,
        before: Option<u64>,
        limit: u32,
    ) -> Result<Vec<CommunityAuditEntry>, GovernanceRuntimeError> {
        let authored = audit::read_audit_entries(self, community_id).await?;
        Ok(audit_page(authored, before, limit))
    }

    /// The community's merged default notification level, if one is set.
    #[must_use]
    pub fn notification_default(&self, community_id: &str) -> Option<String> {
        self.governance_state_impl(community_id)
            .as_ref()
            .and_then(notification_default)
    }

    /// Set the community's default notification level.
    ///
    /// # Errors
    /// The level is unknown, `MANAGE_COMMUNITY` is missing, or the write
    /// failed.
    pub async fn set_notification_default(
        &self,
        community_id: &str,
        level: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        audit::set_notification_default(self, community_id, level).await
    }
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
