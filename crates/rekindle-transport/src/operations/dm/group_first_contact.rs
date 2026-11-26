//! A group member's first contact to a member who is not our friend has a
//! durable copy (plan decision 67 item 3, part 3b; design
//! `design-decision67-rv2c-2026-11-25.md` §3 and F2): the PreKeyInit whose
//! first payload is our group frame (`frame::ratchet_frame_at`, the
//! member-session branch) goes by `app_call` **and** is written, the same
//! datagram, to `subkey(our slot, their slot)` of the group's rendezvous
//! record as our row's one-time writer, through the record pool's
//! republishing set (`write_own_once` → `set_republished`: the stored bytes
//! are re-sent at the same seq until the row is replaced). The member
//! watches its column (`rendezvous_watch`) and opens the copy through the
//! same admission as the call copy (`group_dm::rendezvous::landed` →
//! `direct::receive_from_member`), so a copy arriving both ways is the
//! replay store's or the ratchet's duplicate.
//!
//! Settle rule (F2): the row is named as a first contact (`PreKeyInit:`),
//! so it settles on the call's receipt or on the member's first frame
//! opened under the session (`direct/frame.rs` `open_ratchet` →
//! `handshake::settle_first_contact`), never on the write's own outcome
//! (Veilid answers `Ok(None)` below consensus, `set_value.rs:306-308`). A
//! call that is not receipted leaves the row to be attempted again, the
//! call and the write again; a B6 resend (`group_dm::first_contact`) is a
//! new init, framed here again, whose write replaces the row's value.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmFrame, DmTypeId, FrameBody};
use rekindle_types::dht_layout::group_rendezvous as layout;

use super::handshake::{attempt_call, refuses};
use super::outbox::Attempt;
use super::send::Framed;
use crate::operations::friend::{FriendError, FriendHost};

/// The group a first-payload group frame belongs to, as hex.
fn group_of(body: &DirectBody) -> Option<String> {
    let id = match body {
        DirectBody::GroupDmInvite(f) => f.group_id,
        DirectBody::GroupDmAccept(f) => f.group_id,
        DirectBody::GroupDmDecline(f) => f.group_id,
        DirectBody::GroupDmLeave(f) => f.group_id,
        _ => return None,
    };
    Some(hex::encode(id))
}

/// `framed`, the row of `body` to `to` framed as `frame`, as a group first
/// contact's row when `frame` opens a member session and we hold a place
/// in the group's rendezvous record whose plan gives `to` a slot; the
/// row's kind (`PreKeyInit` for a first contact, else the body's).
///
/// # Errors
/// The datagram does not fit one rendezvous subkey (refused here, never
/// truncated), or our place could not be read.
pub(super) async fn framed(
    host: &dyn FriendHost,
    to: &str,
    body: &DirectBody,
    frame: &DmFrame,
    framed: Framed,
) -> Result<(Framed, DmTypeId), FriendError> {
    let kind = body.type_id();
    let FrameBody::PreKeyInit(init) = &frame.body else {
        return Ok((framed, kind));
    };
    let (Framed::Wire { datagram }, Some(group_id)) = (&framed, group_of(body)) else {
        return Ok((framed, kind));
    };
    if !crate::operations::group_dm::opens_member_session(init.first_type_id)
        || target(host, &group_id, to).await?.is_none()
    {
        return Ok((framed, kind));
    }
    if datagram.len() > layout::MAX_SUBKEY_VALUE_LEN {
        return Err(FriendError::Invalid(format!(
            "a group first contact of {} bytes does not fit a rendezvous subkey ({} bytes)",
            datagram.len(),
            layout::MAX_SUBKEY_VALUE_LEN
        )));
    }
    let datagram = datagram.clone();
    Ok((
        Framed::GroupFirstContact { datagram, group_id },
        DmTypeId::PreKeyInit,
    ))
}

/// Our row of `group_id`'s rendezvous record toward `to`: the record, the
/// subkey, and our row's writer. `None`: we hold no place, or the plan
/// gives `to` no slot.
async fn target(
    host: &dyn FriendHost,
    group_id: &str,
    to: &str,
) -> Result<Option<(String, u32, String)>, FriendError> {
    let id = crate::operations::group_dm::parse_group_id(group_id)?;
    let place = host
        .group_dm()?
        .rendezvous(&id)
        .await
        .map_err(|e| FriendError::Failed(format!("group rendezvous place: {e}")))?;
    Ok(place.and_then(|place| {
        let theirs = place.slots.iter().find(|(m, _)| m == to).map(|(_, s)| *s)?;
        let subkey = layout::subkey(place.slot, theirs)?;
        Some((place.record_key, subkey, place.writer))
    }))
}

/// One attempt of a group first contact: the call and the rendezvous write
/// go together, and only the call's answer is the attempt's outcome (the
/// write never settles the row, F2).
pub(super) async fn attempt(
    host: &dyn FriendHost,
    to: &str,
    datagram: &[u8],
    group_id: &str,
) -> Attempt {
    let wire = Framed::Wire {
        datagram: datagram.to_vec(),
    };
    let kind = match super::send::datagram_of(host, to, &wire) {
        Ok((_, kind)) => kind,
        Err(e) => return Attempt::Wedged(e.to_string()),
    };
    let call = attempt_call(host, to, datagram.to_vec(), kind);
    let (called, ()) = tokio::join!(call, write(host, to, datagram, group_id));
    called
}

/// Write `datagram` to our row's subkey toward `to`, republished. Its
/// outcome is logged only.
async fn write(host: &dyn FriendHost, to: &str, datagram: &[u8], group_id: &str) {
    let target = match target(host, group_id, to).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            tracing::debug!("group first contact: no rendezvous place now; the call goes alone");
            return;
        }
        Err(e) => {
            tracing::debug!(error = %e, "group rendezvous place not read");
            return;
        }
    };
    let Ok(Ok(records)) = host.node().map(|node| node.require_records()) else {
        tracing::debug!("group rendezvous write deferred: the record port is not up");
        return;
    };
    let (record_key, subkey, writer) = target;
    match records
        .write_own_once(&record_key, subkey, datagram.to_vec(), Some(&writer))
        .await
    {
        Ok(outcome) => tracing::debug!(
            ?outcome,
            "group first contact written to its rendezvous row"
        ),
        Err(e) if refuses(&e) => tracing::warn!(error = %e, "group rendezvous write refused"),
        Err(e) => {
            tracing::debug!(error = %e, "group rendezvous write missed — written again at the next attempt");
        }
    }
}

/// One attempt of a frame waiting for its session ([`super::handshake::enqueue_when_ready`]):
/// framed now and committed as its own row, which settles this one; a
/// session still not openable is attempted again; any other failure wedges
/// the row, shown on it.
pub(super) async fn attempt_pending(
    host: &dyn FriendHost,
    to: &str,
    type_id: u16,
    body: &[u8],
) -> Attempt {
    let Some(kind) = DmTypeId::from_u16(type_id) else {
        return Attempt::Wedged(format!("typeId {type_id:#06x}"));
    };
    let body = match DirectBody::decode(kind, body) {
        Ok(body) => body,
        Err(e) => return Attempt::Wedged(format!("queued body: {e}")),
    };
    match super::handshake::enqueue(host, to, &body).await {
        Ok(_) => Attempt::Sent,
        Err(e @ (FriendError::NotReady(_) | FriendError::Locked)) => {
            tracing::debug!(to = %to, ?kind, reason = %e, "frame still waits for the session");
            Attempt::Deferred
        }
        Err(e) => Attempt::Wedged(e.to_string()),
    }
}
