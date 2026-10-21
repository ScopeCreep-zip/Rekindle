//! A group's lifecycle (plan E2.6): create, an invite received, accept,
//! decline, leave, a member's key, a member gone, and our key's rotation.
//! Each returns the frame to send and the members to send it to over their
//! pairwise sessions; the Host adapter (slice G2b) sends them.
//!
//! On any leave, decline or block every remaining member rotates its own
//! key (epoch + 1) and resends it; no member is elected to rotate. A
//! member's first key (its accept) is answered with our current key, so
//! two joiners who accepted in either order both end up holding each
//! other's key.

use rekindle_codec::capnp_codec::direct::{
    GroupDmAccept, GroupDmDecline, GroupDmInvite, GroupDmLeave, GroupId,
};
use rekindle_secrets::group_dm::GroupSenderKey;
use rekindle_types::group_dm::GroupDmRosterMember;
use rekindle_types::subscription_events::{GroupDmEvent, GroupDmMember};

use crate::error::DmError;
use crate::group::{self, group_id_hex, KeyUpdate, Roster};
use crate::ports::{GroupDmDeps, GroupState, PendingInvite};
use crate::sender_key;

/// A frame and the members to send it to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound<F> {
    pub frame: F,
    pub to: Vec<String>,
}

/// A group we created, and the invite to send to every other member.
#[derive(Debug, Clone)]
pub struct Created {
    pub group: GroupState,
    pub invite: Outbound<GroupDmInvite>,
}

async fn member<D: GroupDmDeps + ?Sized>(deps: &D, identity: &str) -> GroupDmMember {
    GroupDmMember {
        public_key: identity.to_owned(),
        display_name: deps.display_name(identity).await,
    }
}

async fn held_group<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group_id: &GroupId,
) -> Result<GroupState, DmError> {
    deps.group(group_id)
        .await?
        .ok_or_else(|| DmError::UnknownGroup(group_id_hex(group_id)))
}

async fn own_key<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group: &GroupState,
) -> Result<GroupSenderKey, DmError> {
    let me = deps.own_identity();
    deps.latest_sender_key(&group.group_id, &me)
        .await?
        .ok_or(DmError::KeyMissing {
            member: me,
            epoch: group.epoch,
        })
}

fn key_frame(group_id: GroupId, key: &GroupSenderKey) -> GroupDmAccept {
    GroupDmAccept {
        group_id,
        epoch: key.epoch(),
        sender_key: *key.as_bytes(),
    }
}

/// Create a group named `name` of `own` (us, the creator, first on the
/// roster) and `others`, with our first sending key.
///
/// # Errors
/// An invalid name or roster, `own` not our identity, or storage.
pub async fn create<D: GroupDmDeps + ?Sized>(
    deps: &D,
    name: &str,
    own: GroupDmRosterMember,
    others: Vec<GroupDmRosterMember>,
) -> Result<Created, DmError> {
    group::check_name(name)?;
    let me = deps.own_identity();
    if own.identity != me {
        return Err(DmError::InvalidRoster("the creator is not us".into()));
    }
    let roster = Roster::new(std::iter::once(own).chain(others).collect())?;
    let group_id = group::new_group_id(deps.rng());
    let key = sender_key::mint_first(deps.rng());
    let invite = group::build_invite(group_id, name, &roster, &key, |m| deps.sign_identity(m));
    let state = GroupState {
        group_id,
        name: name.to_owned(),
        epoch: key.epoch(),
        roster,
        created_at_ms: deps.now_ms(),
    };
    deps.insert_group(&state).await?;
    deps.put_sender_key(&group_id, &me, &key).await?;
    deps.publish(GroupDmEvent::Joined {
        group_id: group_id_hex(&group_id),
        member: member(deps, &me).await,
    });
    let to = state.roster.others(&me);
    Ok(Created {
        group: state,
        invite: Outbound { frame: invite, to },
    })
}

/// A creator's invite over `from`'s session: verified, we are on its
/// roster, and held until we answer it.
///
/// # Errors
/// It does not verify, we are not on it, we already hold the group, or
/// storage.
pub async fn on_invite<D: GroupDmDeps + ?Sized>(
    deps: &D,
    from: &str,
    invite: GroupDmInvite,
) -> Result<(), DmError> {
    let roster = group::verify_invite(&invite, from)?;
    let me = deps.own_identity();
    if !roster.contains(&me) {
        return Err(DmError::NotOnRoster(me));
    }
    if deps.group(&invite.group_id).await?.is_some() {
        return Err(DmError::AlreadyMember(group_id_hex(&invite.group_id)));
    }
    let mut members = Vec::with_capacity(roster.members().len());
    for m in roster.members() {
        members.push(member(deps, &m.identity).await);
    }
    let event = GroupDmEvent::Invited {
        group_id: group_id_hex(&invite.group_id),
        name: invite.name.clone(),
        inviter: member(deps, from).await,
        members,
    };
    deps.stash_invite(&PendingInvite {
        from: from.to_owned(),
        invite,
    })
    .await?;
    deps.publish(event);
    Ok(())
}

/// Accept the pending invite to `group_id`: the group is ours, the
/// creator's key is held, and our first key goes to every other member.
///
/// # Errors
/// No such invite, or storage.
pub async fn accept<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group_id: &GroupId,
) -> Result<Outbound<GroupDmAccept>, DmError> {
    let pending = deps
        .take_invite(group_id)
        .await?
        .ok_or_else(|| DmError::UnknownGroup(group_id_hex(group_id)))?;
    let roster = group::verify_invite(&pending.invite, &pending.from)?;
    let me = deps.own_identity();
    let key = sender_key::mint_first(deps.rng());
    let state = GroupState {
        group_id: *group_id,
        name: pending.invite.name.clone(),
        roster,
        epoch: key.epoch(),
        created_at_ms: deps.now_ms(),
    };
    deps.insert_group(&state).await?;
    let creator_key = GroupSenderKey::new(pending.invite.epoch, pending.invite.creator_sender_key);
    deps.put_sender_key(group_id, &pending.from, &creator_key)
        .await?;
    deps.put_sender_key(group_id, &me, &key).await?;
    deps.publish(GroupDmEvent::Joined {
        group_id: group_id_hex(group_id),
        member: member(deps, &me).await,
    });
    Ok(Outbound {
        frame: key_frame(*group_id, &key),
        to: state.roster.others(&me),
    })
}

/// Decline the pending invite to `group_id`; the decline goes to its
/// creator.
///
/// # Errors
/// No such invite, or storage.
pub async fn decline<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group_id: &GroupId,
) -> Result<Outbound<GroupDmDecline>, DmError> {
    let pending = deps
        .take_invite(group_id)
        .await?
        .ok_or_else(|| DmError::UnknownGroup(group_id_hex(group_id)))?;
    Ok(Outbound {
        frame: GroupDmDecline {
            group_id: *group_id,
        },
        to: vec![pending.from],
    })
}

/// Leave `group_id`: the group and its keys are deleted, and the leave
/// goes to every other member.
///
/// # Errors
/// No such group, or storage.
pub async fn leave<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group_id: &GroupId,
) -> Result<Outbound<GroupDmLeave>, DmError> {
    let group = held_group(deps, group_id).await?;
    let me = deps.own_identity();
    deps.delete_group(group_id).await?;
    deps.publish(GroupDmEvent::Left {
        group_id: group_id_hex(group_id),
        member: member(deps, &me).await,
    });
    Ok(Outbound {
        frame: GroupDmLeave {
            group_id: *group_id,
            epoch: group.epoch,
        },
        to: group.roster.others(&me),
    })
}

/// Our key's next epoch in `group_id`, to every other member.
///
/// # Errors
/// No such group or key, epochs exhausted, or storage.
pub async fn rotate_own_key<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group_id: &GroupId,
) -> Result<Outbound<GroupDmAccept>, DmError> {
    let group = held_group(deps, group_id).await?;
    rotate_in(deps, &group, &group.roster).await
}

async fn rotate_in<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group: &GroupState,
    roster: &Roster,
) -> Result<Outbound<GroupDmAccept>, DmError> {
    let me = deps.own_identity();
    let next = sender_key::rotate(&own_key(deps, group).await?, deps.rng())?;
    deps.put_sender_key(&group.group_id, &me, &next).await?;
    deps.update_group(&group.group_id, roster, next.epoch())
        .await?;
    deps.publish(GroupDmEvent::KeyRotated {
        group_id: group_id_hex(&group.group_id),
        member: member(deps, &me).await,
        epoch: next.epoch(),
    });
    Ok(Outbound {
        frame: key_frame(group.group_id, &next),
        to: roster.others(&me),
    })
}

/// `who` is gone from `group_id` (its leave, its decline, or our block of
/// it): off our roster, its keys deleted, and our key rotated to the rest.
///
/// # Errors
/// No such group, `who` not on it, or storage.
pub async fn remove_member<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group_id: &GroupId,
    who: &str,
) -> Result<Outbound<GroupDmAccept>, DmError> {
    let group = held_group(deps, group_id).await?;
    if who == deps.own_identity() {
        return Err(DmError::InvalidRoster("we leave with leave()".into()));
    }
    let roster = group
        .roster
        .without(who)
        .ok_or_else(|| DmError::NotOnRoster(who.to_owned()))?;
    deps.delete_member_keys(group_id, who).await?;
    deps.publish(GroupDmEvent::Left {
        group_id: group_id_hex(group_id),
        member: member(deps, who).await,
    });
    rotate_in(deps, &group, &roster).await
}

/// A member's `GroupDmLeave` over `from`'s session.
///
/// # Errors
/// As [`remove_member`].
pub async fn on_member_left<D: GroupDmDeps + ?Sized>(
    deps: &D,
    from: &str,
    leave: &GroupDmLeave,
) -> Result<Outbound<GroupDmAccept>, DmError> {
    remove_member(deps, &leave.group_id, from).await
}

/// An invitee's `GroupDmDecline` over `from`'s session: it held our key
/// from the invite, so it leaves as a member does.
///
/// # Errors
/// As [`remove_member`].
pub async fn on_decline<D: GroupDmDeps + ?Sized>(
    deps: &D,
    from: &str,
    decline: &GroupDmDecline,
) -> Result<Outbound<GroupDmAccept>, DmError> {
    remove_member(deps, &decline.group_id, from).await
}

/// A member's sending key (its accept, or a rotation's resend) over
/// `from`'s session. Epochs only move up. A member's first key is a join,
/// answered with our current key to that member.
///
/// # Errors
/// No such group, `from` not on it, an older or conflicting key, or
/// storage.
pub async fn on_member_key<D: GroupDmDeps + ?Sized>(
    deps: &D,
    from: &str,
    frame: &GroupDmAccept,
) -> Result<Option<Outbound<GroupDmAccept>>, DmError> {
    let group = held_group(deps, &frame.group_id).await?;
    if !group.roster.contains(from) || from == deps.own_identity() {
        return Err(DmError::NotOnRoster(from.to_owned()));
    }
    let incoming = GroupSenderKey::new(frame.epoch, frame.sender_key);
    let held = deps.latest_sender_key(&frame.group_id, from).await?;
    if group::key_update(held.as_ref(), &incoming)? == KeyUpdate::Duplicate {
        return Ok(None);
    }
    deps.put_sender_key(&frame.group_id, from, &incoming)
        .await?;
    let gid = group_id_hex(&frame.group_id);
    if held.is_some() {
        deps.publish(GroupDmEvent::KeyRotated {
            group_id: gid,
            member: member(deps, from).await,
            epoch: incoming.epoch(),
        });
        return Ok(None);
    }
    deps.publish(GroupDmEvent::Joined {
        group_id: gid,
        member: member(deps, from).await,
    });
    let ours = own_key(deps, &group).await?;
    Ok(Some(Outbound {
        frame: key_frame(frame.group_id, &ours),
        to: vec![from.to_owned()],
    }))
}

/// Seal `plaintext` (a content kind or typing) as our entry in
/// `group_id`, under our current key.
///
/// # Errors
/// No such group or key, the cipher, or storage.
pub async fn seal_for_send<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group_id: &GroupId,
    plaintext: &[u8],
) -> Result<rekindle_codec::capnp_codec::direct::GroupContent, DmError> {
    let group = held_group(deps, group_id).await?;
    let key = own_key(deps, &group).await?;
    sender_key::seal(&key, *group_id, &deps.own_identity(), plaintext)
}

/// Open `from`'s entry: `from` on the roster, the entry at its current
/// epoch (older is refused, newer waits for its key).
///
/// # Errors
/// No such group, `from` not on it, the epoch, the cipher, or storage.
pub async fn open_received<D: GroupDmDeps + ?Sized>(
    deps: &D,
    from: &str,
    content: &rekindle_codec::capnp_codec::direct::GroupContent,
) -> Result<Vec<u8>, DmError> {
    let group = held_group(deps, &content.group_id).await?;
    if !group.roster.contains(from) {
        return Err(DmError::NotOnRoster(from.to_owned()));
    }
    let key = deps
        .latest_sender_key(&content.group_id, from)
        .await?
        .ok_or_else(|| DmError::KeyMissing {
            member: from.to_owned(),
            epoch: content.epoch,
        })?;
    sender_key::open(&key, from, content)
}

#[cfg(test)]
#[path = "lifecycle/tests.rs"]
mod tests;
