//! Group DMs on both hosts (plan E2.6 G2b): the Host-facing operations
//! over `rekindle-dm`'s lifecycle (through `FriendHost::group_dm`), their
//! pairwise control frames, the receive arms and the group admission.
//!
//! Every lifecycle step returns a frame and the members it goes to; each
//! goes over that member's pairwise session as a `friend_handshake` outbox
//! row ([`crate::operations::dm::handshake::enqueue_when_ready`]), sent
//! until it settles and survives a restart. The keys inside are never
//! printed.

mod admission;
pub mod content;
mod inbound;
mod pairwise;
pub mod receive;
mod rename;
pub mod retire;
mod review;
mod room;
pub use rename::rename;
pub use room::create_from;
mod view;

pub use admission::admitted;
pub use content::{report_draft, send};
pub use inbound::apply;
pub(crate) use inbound::on_stream_record;
pub(crate) use pairwise::member_profile;
pub use pairwise::GroupFirstContact;
pub(crate) use pairwise::{member_bundle, opens_member_session};
pub use review::{key_changes, send_reviewed};
pub use view::{find, list, mark_read, messages};
pub(crate) mod first_contact {
    //! The group first contact, as the prekey rejection flow sees it.
    pub(crate) use super::pairwise::{acknowledged, began, on_rejected};
}

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupId};
use rekindle_dm::{DmError, Outbound};

use crate::operations::dm::handshake;
use rekindle_types::group_dm::GroupDmRosterMember;

use crate::operations::friend::{state, FriendError, FriendHost};

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
        DmError::ChainMissing(_) => FriendError::NotReady(e.to_string()),
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
/// A member whose session cannot be opened yet (their prekey record not
/// readable) gets the frame committed to wait for it
/// ([`handshake::enqueue_when_ready`], finding GA1): the step is committed
/// at once and sent when the session opens. A member whose row could not
/// be framed or stored does not stop the rest; the first such error is
/// returned once all were tried.
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
        if let Err(e) = handshake::enqueue_when_ready(host, to, &body).await {
            tracing::warn!(to = %to, kind = ?body.type_id(), error = %e,
                "group frame not committed to this member");
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Create a group named `name` of us and `members` (our friends, 2-7 of
/// them): our chain first, then the signed invite to each member. The new
/// group's id.
///
/// # Errors
/// A member who is not a friend (we need their profile record), an
/// invalid name or roster, the chain record, storage, or a member's row.
pub async fn create(
    host: &dyn FriendHost,
    name: &str,
    members: &[String],
) -> Result<String, FriendError> {
    rekindle_dm::group::check_name(name).map_err(|e| friend_error(&e))?;
    // The roster names our profile: wait for the own-records stage to
    // hold it (live finding 24), as a friend request does.
    let own = crate::operations::friend::published_identity(host).await?;
    let others = friend_members(host, members)?;
    let me = GroupDmRosterMember {
        identity: own.public_key,
        profile_record_key: own.profile_dht_key,
        display_name: own.display_name,
    };
    let group_id = rekindle_dm::group::new_group_id(host.rng());
    let gid = rekindle_dm::group::group_id_hex(&group_id);
    let chain = host.group_chain_writer(&gid).await?;
    let deps = host.group_dm()?;
    let created = rekindle_dm::create(deps.as_ref(), group_id, name, me, others, &chain.record_key)
        .await
        .map_err(|e| friend_error(&e))?;
    send_pairwise(host, created.invite, DirectBody::GroupDmInvite).await?;
    Ok(gid)
}

/// Grow `group_id`'s roster by `members` (our friends; plan E7.2 G8,
/// Xfire's "Invite users to room"): only its creator, up to eight. The
/// invite goes to each newcomer, who accepts or declines it on the E2.6
/// path, and the grow to each member already on it.
///
/// # Errors
/// Not a group we hold, we are not its creator, it would pass eight, a
/// member who is not a friend or is already on it, storage, or a row.
pub async fn invite(
    host: &dyn FriendHost,
    group_id: &str,
    members: &[String],
) -> Result<(), FriendError> {
    let id = parse_group_id(group_id)?;
    let deps = host.group_dm()?;
    let held = content::held(host, &id).await?;
    if let Some(reason) = rekindle_dm::invite_refused(&held.roster, &deps.own_identity()) {
        return Err(FriendError::Invalid(reason));
    }
    let max = rekindle_types::group_dm::MAX_MEMBERS;
    if held.roster.members().len() + members.len() > max {
        return Err(FriendError::Invalid(format!(
            "a group holds at most {max} members"
        )));
    }
    let added = friend_members(host, members)?;
    let grown = rekindle_dm::grow(deps.as_ref(), &id, added)
        .await
        .map_err(|e| friend_error(&e))?;
    let invited = send_pairwise(host, grown.invite, DirectBody::GroupDmInvite).await;
    let told = send_pairwise(host, grown.grow, DirectBody::GroupDmRosterGrow).await;
    invited.and(told)
}

/// `members` (each a friend the picker allows, GC3) as roster entries.
fn friend_members(
    host: &dyn FriendHost,
    members: &[String],
) -> Result<Vec<GroupDmRosterMember>, FriendError> {
    let mut others = Vec::with_capacity(members.len());
    for key in members {
        let friend = state::friend(host, key).ok_or_else(|| {
            FriendError::Invalid(format!("{} is not a friend", state::short_key(key)))
        })?;
        // The picker's own check (plan E2.6, GC3): it shows this reason.
        if let Some(reason) = rekindle_types::dto::friends::group_dm_block(&friend) {
            return Err(FriendError::Invalid(format!(
                "{}: {reason}",
                friend.display_name
            )));
        }
        let (profile, display_name) = friend
            .dht_record_key
            .map(|profile| (profile, friend.display_name))
            .ok_or_else(|| FriendError::Invalid("no profile record".into()))?;
        others.push(GroupDmRosterMember {
            identity: key.clone(),
            profile_record_key: profile,
            display_name,
        });
    }
    Ok(others)
}

/// Accept the pending invite to `group_id`: our chain first, then our
/// first key and chain go to every other member.
///
/// # Errors
/// No such invite, the chain record, storage, or a member's row.
pub async fn accept(host: &dyn FriendHost, group_id: &str) -> Result<(), FriendError> {
    let id = parse_group_id(group_id)?;
    let chain = host.group_chain_writer(group_id).await?;
    let deps = host.group_dm()?;
    let out = rekindle_dm::accept(deps.as_ref(), &id, &chain.record_key)
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
/// every other member, who each rotate their own key. Each leave is framed
/// when its row is committed, so the pairwise session with a member who
/// is not our friend (and in no other group we hold) then ends.
///
/// # Errors
/// No such group, storage, or a member's row.
pub async fn leave(host: &dyn FriendHost, group_id: &str) -> Result<(), FriendError> {
    let deps = host.group_dm()?;
    let out = rekindle_dm::leave(deps.as_ref(), &parse_group_id(group_id)?)
        .await
        .map_err(|e| friend_error(&e))?;
    let members = out.to.clone();
    let sent = send_pairwise(host, out, DirectBody::GroupDmLeave).await;
    pairwise::drop_sessions(host, &members).await;
    sent
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
