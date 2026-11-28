//! Growing a group's roster (plan E7.2 G8, SK2-G §6): only the creator,
//! whose signature makes a roster (E2.6), invites more of its friends, up
//! to `MAX_MEMBERS`. The creator rotates its key to the next epoch and
//! re-signs the invite with the grown roster (the held one as its prefix):
//! the newcomers get it as a `GroupDmInvite` they accept or decline on the
//! E2.6 path, and every member already on the roster gets it inside a
//! `GroupDmRosterGrow`. Each such member stores the grown roster and the
//! creator's new key and rotates its own key to the members it already
//! had, as on a leave; a newcomer gets each member's rotated key when its
//! accept arrives ([`super::on_member_key`]'s join reply). Every member's
//! key a newcomer holds is newer than the grow, so it reads none of the
//! history (Signal's group rule).

use rekindle_codec::capnp_codec::direct::{
    GroupDmAccept, GroupDmInvite, GroupDmRosterGrow, GroupId,
};
use rekindle_secrets::group_dm::GroupSenderKey;
use rekindle_types::group_dm::GroupDmRosterMember;
use rekindle_types::subscription_events::GroupDmEvent;

use super::{held_group, member, own_chain, own_key, rotate_in, Outbound};
use crate::error::DmError;
use crate::group::{self, group_id_hex, KeyUpdate, Roster};
use crate::ports::{GroupDmDeps, PendingInvite, RendezvousMint, RendezvousSlot};
use crate::sender_key;
use rekindle_types::dht_layout::group_rendezvous as rendezvous;

/// The grown roster's two frames: the invite to the newcomers, and the
/// grow to every member already on the roster.
#[derive(Debug, Clone)]
pub struct Grown {
    pub invites: Vec<Outbound<GroupDmInvite>>,
    pub grows: Vec<Outbound<GroupDmRosterGrow>>,
}

/// The grown roster's slots (plan decision 67 item 3): a member already on
/// it keeps its slot, and each newcomer, in roster order, takes the lowest
/// row no current member holds (eight rows for at most eight members,
/// `group_dm::MAX_MEMBERS`). A departed member's row is free again: it
/// still holds that row's writer, so it can junk-write the row, never
/// forge an entry in it (the reader takes row `i` only from the member
/// the plan assigns it).
fn grown_slots(held: &RendezvousSlot, roster: &Roster) -> Result<Vec<u8>, DmError> {
    let mut taken = 0u16;
    let mut slots: Vec<Option<u8>> = roster
        .members()
        .iter()
        .map(|m| {
            let kept = held
                .slots
                .iter()
                .find(|(id, _)| *id == m.identity)
                .map(|(_, s)| *s);
            if let Some(s) = kept {
                taken |= 1 << s;
            }
            kept
        })
        .collect();
    for slot in &mut slots {
        if slot.is_none() {
            let free = (0..rendezvous::WRITERS)
                .find(|s| taken & (1 << s) == 0)
                .ok_or_else(|| DmError::InvalidRoster("no free rendezvous row".into()))?;
            taken |= 1 << free;
            *slot = Some(free);
        }
    }
    Ok(slots.into_iter().flatten().collect())
}

/// Invite `added` (the creator's friends, none on the roster) to
/// `group_id`: our key moves to the next epoch, the grown roster is stored
/// and signed, and the frames go out.
///
/// # Errors
/// No such group, we are not its creator, it is full, nobody to add, a
/// member already on it, a roster past `MAX_MEMBERS`, epochs exhausted, or
/// storage.
pub async fn grow<D: GroupDmDeps + ?Sized>(
    deps: &D,
    group_id: &GroupId,
    added: Vec<GroupDmRosterMember>,
    mint: &RendezvousMint,
) -> Result<Grown, DmError> {
    let group = held_group(deps, group_id).await?;
    let me = deps.own_identity();
    if let Some(reason) = group::invite_refused(&group.roster, &me) {
        return Err(DmError::InvalidRoster(reason));
    }
    if added.is_empty() {
        return Err(DmError::InvalidRoster("nobody to invite".into()));
    }
    if let Some(on) = added.iter().find(|m| group.roster.contains(&m.identity)) {
        return Err(DmError::AlreadyMember(on.identity.clone()));
    }
    let had = group.roster.others(&me);
    let roster = Roster::new(
        group
            .roster
            .members()
            .iter()
            .cloned()
            .chain(added)
            .collect(),
    )?;
    let next = sender_key::rotate(&own_key(deps, &group).await?, deps.rng())?;
    let chain = own_chain(deps, group_id).await?;
    let held = deps
        .rendezvous(group_id)
        .await?
        .ok_or_else(|| DmError::InvalidRoster("the group has no rendezvous record".into()))?;
    let slots = grown_slots(&held, &roster)?;
    let plan = group::RendezvousPlan {
        record_key: &held.record_key,
        slots: &slots,
    };
    let invite = group::build_invite(*group_id, &group.name, &roster, &next, &chain, plan, |m| {
        deps.sign_identity(m)
    })?;
    let own = group::for_recipient(&invite, &me, mint)?;
    deps.put_rendezvous(group_id, &group::rendezvous_of(&own, &me)?)
        .await?;
    deps.put_sender_key(group_id, &me, &next).await?;
    deps.update_group(group_id, &roster, next.epoch()).await?;
    deps.publish(GroupDmEvent::KeyRotated {
        group_id: group_id_hex(group_id),
        member: member(deps, &me).await,
        epoch: next.epoch(),
    });
    let newcomers = roster
        .others(&me)
        .into_iter()
        .filter(|m| !had.contains(m))
        .collect();
    let grows = super::per_recipient(&invite, had, mint)?
        .into_iter()
        .map(|out| Outbound {
            frame: GroupDmRosterGrow { invite: out.frame },
            to: out.to,
        })
        .collect();
    Ok(Grown {
        grows,
        invites: super::per_recipient(&invite, newcomers, mint)?,
    })
}

/// `grown` extends `held`: the held members first, in order, and more.
fn extends(held: &Roster, grown: &Roster) -> Result<(), DmError> {
    let (h, g) = (held.members(), grown.members());
    if g.len() > h.len() && g[..h.len()] == *h {
        Ok(())
    } else {
        Err(DmError::InvalidRoster(
            "the grown roster does not extend the held one".into(),
        ))
    }
}

/// The creator's `GroupDmRosterGrow` over `from`'s session. For a group we
/// hold: the grown roster and the creator's new key and chain are stored,
/// and our key, rotated, goes to the members we already had (`None` when
/// it is the grow we already applied). For an invite we hold unanswered:
/// the grown invite replaces it.
///
/// # Errors
/// It does not verify as an invite, `from` is not the creator, we are not
/// on it, it does not extend the held roster, an older creator key, no
/// such group or invite, or storage.
pub async fn on_roster_grow<D: GroupDmDeps + ?Sized>(
    deps: &D,
    from: &str,
    grow: &GroupDmRosterGrow,
) -> Result<Option<Outbound<GroupDmAccept>>, DmError> {
    let invite = &grow.invite;
    let roster = group::verify_invite(invite, from)?;
    let me = deps.own_identity();
    if !roster.contains(&me) {
        return Err(DmError::NotOnRoster(me));
    }
    let gid = invite.group_id;
    let Some(group) = deps.group(&gid).await? else {
        return replace_pending(deps, from, invite, &roster)
            .await
            .map(|()| None);
    };
    if group.roster.creator().identity != from {
        return Err(DmError::NotOnRoster(from.to_owned()));
    }
    let incoming = GroupSenderKey::new(invite.epoch, invite.creator_sender_key);
    let held = deps.latest_sender_key(&gid, from).await?;
    let update = group::key_update(held.as_ref(), &incoming)?;
    if update == KeyUpdate::Duplicate && group.roster == roster {
        return Ok(None);
    }
    extends(&group.roster, &roster)?;
    let held = deps
        .rendezvous(&gid)
        .await?
        .ok_or_else(|| DmError::InvalidRoster("the group has no rendezvous record".into()))?;
    let place = group::rendezvous_of(invite, &me)?;
    if place.record_key != held.record_key || place.slot != held.slot {
        return Err(DmError::InvalidRoster(
            "a grow moved our rendezvous record or slot".into(),
        ));
    }
    deps.put_rendezvous(
        &gid,
        &RendezvousSlot {
            writer: held.writer,
            ..place
        },
    )
    .await?;
    deps.put_sender_key(&gid, from, &incoming).await?;
    deps.put_chain(&gid, from, &invite.creator_chain_key)
        .await?;
    deps.publish(GroupDmEvent::KeyRotated {
        group_id: group_id_hex(&gid),
        member: member(deps, from).await,
        epoch: incoming.epoch(),
    });
    let had = group.roster.others(&me);
    let mut out = rotate_in(deps, &group, &roster).await?;
    out.to.retain(|m| had.contains(m));
    Ok(Some(out))
}

/// The grown invite replaces the unanswered one from the same creator,
/// when it extends that one's roster; the held one stays otherwise.
async fn replace_pending<D: GroupDmDeps + ?Sized>(
    deps: &D,
    from: &str,
    invite: &GroupDmInvite,
    roster: &Roster,
) -> Result<(), DmError> {
    let pending = deps
        .take_invite(&invite.group_id)
        .await?
        .ok_or_else(|| DmError::UnknownGroup(group_id_hex(&invite.group_id)))?;
    let checked = group::verify_invite(&pending.invite, &pending.from).and_then(|held| {
        if pending.from != from {
            return Err(DmError::NotOnRoster(from.to_owned()));
        }
        extends(&held, roster)
    });
    let kept = match checked {
        Ok(()) => PendingInvite {
            from: from.to_owned(),
            invite: invite.clone(),
        },
        Err(_) => pending,
    };
    deps.stash_invite(&kept).await?;
    checked
}
