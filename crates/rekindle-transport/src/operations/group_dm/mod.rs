//! Group DMs on both hosts (plan E2.6 G2b): the Host-facing operations
//! over `rekindle-dm`'s lifecycle (through `FriendHost::group_dm`), their
//! pairwise control frames, the receive arms and the group admission.
//!
//! Every lifecycle step returns a frame and the members it goes to; each
//! goes over that member's pairwise session as a `friend_handshake` outbox
//! row ([`crate::operations::dm::handshake::enqueue`]), so it is sent
//! until it settles and survives a restart. The keys inside are never
//! printed.

mod admission;
mod inbound;

pub use admission::admitted;
pub use inbound::apply;

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupId};
use rekindle_dm::{DmError, Outbound};

use crate::operations::dm::handshake;
use crate::operations::friend::{FriendError, FriendHost};

/// A lifecycle error, as the operations report it.
pub(crate) fn friend_error(e: &DmError) -> FriendError {
    match e {
        DmError::UnknownGroup(_) => FriendError::NotFound(e.to_string()),
        DmError::AlreadyMember(_) | DmError::StaleEpoch { .. } | DmError::ConflictingKey(_) => {
            FriendError::Conflict(e.to_string())
        }
        DmError::Storage(_) | DmError::Crypto(_) | DmError::EpochExhausted => {
            FriendError::Failed(e.to_string())
        }
        DmError::InvalidRoster(_)
        | DmError::InvalidName(_)
        | DmError::BadSignature
        | DmError::NotOnRoster(_)
        | DmError::KeyMissing { .. } => FriendError::Invalid(e.to_string()),
    }
}

/// A group id as a frontend names it: 32 lowercase hex.
///
/// # Errors
/// `Invalid` when it is not.
pub fn parse_group_id(text: &str) -> Result<GroupId, FriendError> {
    let bytes =
        hex::decode(text).map_err(|_| FriendError::Invalid("group id is not hex".into()))?;
    bytes
        .try_into()
        .map_err(|_| FriendError::Invalid("group id is not 16 bytes".into()))
}

/// Commit `out`'s frame to every member it names, one handshake row each.
/// A member whose row could not be framed or stored does not stop the
/// rest; the first such error is returned once all were tried.
///
/// # Errors
/// A row to some member could not be framed or stored.
pub(crate) async fn send_pairwise<F>(
    host: &dyn FriendHost,
    out: Outbound<F>,
    wrap: fn(F) -> DirectBody,
) -> Result<(), FriendError> {
    let body = wrap(out.frame);
    let mut first_error = None;
    for to in &out.to {
        if let Err(e) = handshake::enqueue(host, to, &body).await {
            tracing::warn!(to = %to, kind = ?body.type_id(), error = %e,
                "group frame not committed to this member");
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Accept the pending invite to `group_id`; our first key goes to every
/// other member.
///
/// # Errors
/// No such invite, storage, or a member's row.
pub async fn accept(host: &dyn FriendHost, group_id: &str) -> Result<(), FriendError> {
    let deps = host.group_dm()?;
    let out = rekindle_dm::accept(deps.as_ref(), &parse_group_id(group_id)?)
        .await
        .map_err(|e| friend_error(&e))?;
    send_pairwise(host, out, DirectBody::GroupDmAccept).await
}

/// Decline the pending invite to `group_id`; the decline goes to its
/// creator.
///
/// # Errors
/// No such invite, storage, or the creator's row.
pub async fn decline(host: &dyn FriendHost, group_id: &str) -> Result<(), FriendError> {
    let deps = host.group_dm()?;
    let out = rekindle_dm::decline(deps.as_ref(), &parse_group_id(group_id)?)
        .await
        .map_err(|e| friend_error(&e))?;
    send_pairwise(host, out, DirectBody::GroupDmDecline).await
}

/// Leave `group_id`: our state of it is deleted and the leave goes to
/// every other member, who each rotate their own key.
///
/// # Errors
/// No such group, storage, or a member's row.
pub async fn leave(host: &dyn FriendHost, group_id: &str) -> Result<(), FriendError> {
    let deps = host.group_dm()?;
    let out = rekindle_dm::leave(deps.as_ref(), &parse_group_id(group_id)?)
        .await
        .map_err(|e| friend_error(&e))?;
    send_pairwise(host, out, DirectBody::GroupDmLeave).await
}

/// We blocked `peer` (`friend::block`, whose transaction dropped our
/// sessions with them): off every group's roster we share, its keys
/// deleted, and our key rotated to the rest of each.
///
/// # Errors
/// The groups could not be read; a group's rotation or a member's row.
pub async fn on_blocked(host: &dyn FriendHost, peer: &str) -> Result<(), FriendError> {
    let deps = host.group_dm()?;
    let groups = deps.groups().await.map_err(|e| friend_error(&e))?;
    let mut first_error = None;
    for group in groups.iter().filter(|g| g.roster.contains(peer)) {
        let sent = match rekindle_dm::remove_member(deps.as_ref(), &group.group_id, peer).await {
            Ok(out) => send_pairwise(host, out, DirectBody::GroupDmAccept).await,
            Err(e) => Err(friend_error(&e)),
        };
        if let Err(e) = sent {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}
