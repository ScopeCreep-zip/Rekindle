//! A send to a group is reviewed as a direct send is (plan E2.6, audit
//! R-4): each other member whose verified key changed and is not yet
//! acknowledged escalates the review, and a send whose review was held on
//! those changes acknowledges each before the message is committed. Only
//! a friend has a verified key, so a member who is not one holds nothing.

use rekindle_types::intent::Escalation;

use super::{find, send};
use crate::operations::friend::{state, verification, FriendError, FriendHost};

/// Every other member's key, by the group's roster.
async fn others(host: &dyn FriendHost, group_id: &str) -> Result<Vec<String>, FriendError> {
    let owner = state::owner(host)?;
    let view = find(host, group_id)
        .await?
        .ok_or_else(|| FriendError::NotFound(format!("no group {group_id}")))?;
    Ok(view
        .members
        .into_iter()
        .map(|m| m.key)
        .filter(|key| *key != owner)
        .collect())
}

/// Each other member's `VerifiedKeyChanged`, in roster order.
///
/// # Errors
/// No such group, or a member's verification could not be read.
pub async fn key_changes(
    host: &dyn FriendHost,
    group_id: &str,
) -> Result<Vec<Escalation>, FriendError> {
    let mut out = Vec::new();
    for member in others(host, group_id).await? {
        out.extend(verification::key_change(host, &member).await?);
    }
    Ok(out)
}

/// Send `body` to `group_id` ([`send`]); a send whose review was held on
/// members' key changes (`acknowledges_key_changes`) acknowledges each
/// first, so their frames, the hint included, are not refused.
///
/// # Errors
/// An acknowledgement or the send failed.
pub async fn send_reviewed(
    host: &dyn FriendHost,
    group_id: &str,
    body: &str,
    acknowledges_key_changes: bool,
) -> Result<String, FriendError> {
    if acknowledges_key_changes {
        for member in others(host, group_id).await? {
            if verification::key_change(host, &member).await?.is_some() {
                verification::acknowledge_key_change(host, &member).await?;
            }
        }
    }
    send(host, group_id, body).await
}
