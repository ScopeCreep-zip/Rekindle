//! The community audit log and the community notification default (plan
//! E3.1 F4).
//!
//! The audit log is the community's governance log itself: every entry is
//! signed by its author's pseudonym (W26), so it is read back from the
//! governance record's occupied subkeys with every author's overflow chain
//! followed, the same reader the hydration uses. A failed inspect is an
//! error: there is no blind `0..255` scan (plan C7.5).
//!
//! The notification default (architecture §17.1 tier 2) is a
//! `CommunityNotificationDefault` governance entry readers accept only from a
//! member holding `MANAGE_COMMUNITY`, with one of the three levels.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{MANAGE_COMMUNITY, VIEW_AUDIT_LOG};

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// The levels readers accept for a community notification default.
pub const NOTIFICATION_LEVELS: [&str; 3] = ["all", "mentions", "nothing"];

/// Every verified governance entry of `community_id`, by author. Requires
/// `VIEW_AUDIT_LOG`.
///
/// # Errors
/// The permission is missing, or the record could not be inspected.
pub async fn read_audit_entries<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<Vec<(PseudonymKey, Vec<GovernanceEntry>)>, GovernanceRuntimeError> {
    deps.require_permission(community_id, VIEW_AUDIT_LOG)?;
    let occupied = crate::records::inspect_present_subkeys(deps, community_id).await?;
    Ok(
        crate::overflow::read_governance_with_overflow(deps, community_id, &occupied)
            .await
            .authored,
    )
}

/// Write the community-wide default notification level. Requires
/// `MANAGE_COMMUNITY`.
///
/// # Errors
/// The level is not one readers accept, the permission is missing, or the
/// write failed.
pub async fn set_notification_default<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    level: &str,
) -> Result<(), GovernanceRuntimeError> {
    if !NOTIFICATION_LEVELS.contains(&level) {
        return Err(GovernanceRuntimeError::Adapter(format!(
            "unknown notification level {level:?}"
        )));
    }
    deps.require_permission(community_id, MANAGE_COMMUNITY)?;
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::CommunityNotificationDefault {
            level: level.to_owned(),
        },
    )
    .await
}
