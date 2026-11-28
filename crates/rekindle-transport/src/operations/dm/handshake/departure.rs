//! Group departures retain the signed route profile after local deletion.

use rekindle_codec::capnp_codec::direct::{ContentHint, DirectBody, DmTypeId};
use rekindle_types::group_dm::GroupDmRosterMember;

use super::{attempt_call, attempt_call_using, client_id, commit, encode};
use crate::operations::dm::{frame, friend_control, outbox::Attempt, Framed};
use crate::operations::friend::{FriendError, FriendHost};

pub(super) async fn attempt_receipted(
    host: &dyn FriendHost,
    to: &str,
    datagram: Vec<u8>,
    kind: DmTypeId,
    framed: &Framed,
) -> Attempt {
    let Framed::DepartureWire {
        profile_record_key, ..
    } = framed
    else {
        return attempt_call(host, to, datagram, kind).await;
    };
    let outcome = attempt_call_using(host, to, datagram, kind, Some(profile_record_key)).await;
    if matches!(outcome, Attempt::Receipted | Attempt::Superseded(_)) {
        crate::operations::group_dm::drop_sessions(host, &[to.to_owned()]).await;
    }
    outcome
}

/// Queue a departure locally. The outbox opens or uses the pairwise
/// session after the group or invite has been removed.
pub(crate) async fn enqueue_departure(
    host: &dyn FriendHost,
    member: GroupDmRosterMember,
    body: &DirectBody,
) -> Result<String, FriendError> {
    if let Some(id) = friend_control::to_chain(host, &member.identity, body).await? {
        return Ok(id);
    }
    let kind = body.type_id();
    let payload = encode(&Framed::Departure {
        type_id: kind.as_u16(),
        body: body.encode()?,
        profile_record_key: member.profile_record_key,
    })?;
    commit(host, &member.identity, client_id(host, kind), payload).await
}

pub(super) async fn frame_departure(
    host: &dyn FriendHost,
    to: &str,
    type_id: u16,
    body: &[u8],
    profile: &str,
) -> Attempt {
    let Some(kind) = DmTypeId::from_u16(type_id) else {
        return Attempt::Wedged(format!("typeId {type_id:#06x}"));
    };
    let framed = async {
        let body = DirectBody::decode(kind, body)?;
        let frame = frame::ratchet_frame_using(
            host,
            to,
            &body,
            ContentHint::Default,
            frame::Position::default(),
            host.clock().now_ms(),
            Some(profile),
        )
        .await?;
        let payload = encode(&Framed::DepartureWire {
            datagram: frame::wire(&frame)?,
            profile_record_key: profile.to_owned(),
        })?;
        commit(host, to, client_id(host, kind), payload).await?;
        Ok::<_, FriendError>(())
    };
    match framed.await {
        Ok(()) => Attempt::Sent,
        Err(FriendError::NotReady(_) | FriendError::Locked) => Attempt::Deferred,
        Err(e) => Attempt::Wedged(e.to_string()),
    }
}
