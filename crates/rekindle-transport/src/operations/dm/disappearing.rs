//! Disappearing messages (plan E7.1), Signal's mode: each DM and group DM
//! has one timer, set by any member with a `DisappearingTimer` (0x0160)
//! that every member applies and shows as a timeline notice; each message
//! carries the timer in force when it was sent (`DmText.expire_seconds`),
//! so a change racing a message cannot leave two copies on different
//! timers. Our own copy expires at send + timer, a received one at first
//! display + timer. The Host's expiry task ([`expire_due`]) deletes what
//! is due and publishes `MessagesExpired`.

use rekindle_codec::capnp_codec::direct::{DirectBody, DisappearingTimer};
use rekindle_db::repo::disappearing;
use rekindle_types::disappearing::valid;
use rekindle_types::message::ConversationType;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::{client_id_text, handshake, mint_client_id};
use crate::operations::friend::{state, FriendError, FriendHost};
use crate::operations::group_dm::parse_group_id;

/// The timer on `conversation` (the peer, or a group's id), which our next
/// message carries; 0 when off or unreadable.
pub(crate) async fn timer(
    host: &dyn FriendHost,
    conversation: &str,
    kind: ConversationType,
) -> u32 {
    let Ok(owner) = state::owner(host) else {
        return 0;
    };
    let id = conversation.to_owned();
    state::db_call!(host, "read the disappearing timer", move |conn| {
        disappearing::timer(conn, &owner, &id, kind)
    })
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "disappearing timer unreadable; message sent with none");
        0
    })
}

/// Set the timer on our DM with `conversation` or our group DM
/// `conversation` (32 hex) to `seconds` (0 = off), tell every other
/// member, and publish the notice.
///
/// # Errors
/// `Invalid` past four weeks or for a channel, `NotFound` with no such
/// friend or group, or the storage failed.
pub async fn set(
    host: &dyn FriendHost,
    conversation: &str,
    kind: ConversationType,
    seconds: u32,
) -> Result<(), FriendError> {
    if !valid(seconds) {
        return Err(FriendError::Invalid(format!(
            "timer {seconds} s is past four weeks"
        )));
    }
    let owner = state::owner(host)?;
    let (group_id, to) = match kind {
        ConversationType::Dm => (None, vec![conversation.to_owned()]),
        ConversationType::GroupDm => {
            let id = parse_group_id(conversation)?;
            let group = host
                .group_dm()?
                .group(&id)
                .await
                .map_err(|e| crate::operations::group_dm::friend_error(&e))?
                .ok_or_else(|| FriendError::NotFound(format!("no group {conversation}")))?;
            (Some(id), group.roster.others(&owner))
        }
        ConversationType::Channel => {
            return Err(FriendError::Invalid(
                "a channel's timer is a governance op".into(),
            ));
        }
    };
    let (o, id) = (owner.clone(), conversation.to_owned());
    let held = state::db_call!(host, "store the disappearing timer", move |conn| {
        disappearing::set_timer(conn, &o, &id, kind, seconds)
    })
    .await?;
    if !held {
        return Err(FriendError::NotFound(format!(
            "no conversation {conversation}"
        )));
    }
    let body = DirectBody::DisappearingTimer(DisappearingTimer { seconds, group_id });
    let mut first_error = None;
    for peer in &to {
        let sent = if group_id.is_some() {
            handshake::enqueue(host, peer, &body).await.map(|_| ())
        } else {
            // On the DM stream, in order with our messages to them.
            let payload = super::outbox::payload(&body)?;
            let client_id = client_id_text(&mint_client_id(host));
            host.outbox()?
                .enqueue_control(peer, client_id, payload)
                .await
                .map(|_| ())
        };
        if let Err(e) = sent {
            tracing::warn!(to = %peer, error = %e, "disappearing timer not committed to this member");
            first_error.get_or_insert(e);
        }
    }
    publish_change(host, conversation, kind, seconds, &owner);
    first_error.map_or(Ok(()), Err)
}

fn publish_change(
    host: &dyn FriendHost,
    conversation: &str,
    kind: ConversationType,
    seconds: u32,
    set_by: &str,
) {
    host.publish(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DisappearingTimerChanged {
            conversation_id: conversation.to_owned(),
            conversation_type: kind,
            seconds,
            set_by: set_by.to_owned(),
        },
    ));
}

/// Apply `sender`'s `DisappearingTimer`: a friend's sets our DM with them,
/// a group's only from a member on the group's roster. Whether it stored
/// (a refused one is judged and stores nothing).
pub(crate) async fn on_timer(host: &dyn FriendHost, sender: &str, t: &DisappearingTimer) -> bool {
    let (conversation, kind) = match &t.group_id {
        None => (sender.to_owned(), ConversationType::Dm),
        Some(id) => {
            let member = match host.group_dm() {
                Ok(deps) => deps.group(id).await.ok().flatten(),
                Err(_) => return false,
            }
            .is_some_and(|g| g.roster.contains(sender));
            if !member {
                tracing::info!(from = %sender, "disappearing timer from a non-member refused");
                return true;
            }
            (hex::encode(id), ConversationType::GroupDm)
        }
    };
    let Ok(owner) = state::owner(host) else {
        return false;
    };
    let (seconds, id) = (t.seconds, conversation.clone());
    match state::db_call!(host, "store the peer's disappearing timer", move |conn| {
        disappearing::set_timer(conn, &owner, &id, kind, seconds)
    })
    .await
    {
        Ok(true) => {
            publish_change(host, &conversation, kind, t.seconds, sender);
            true
        }
        Ok(false) => true,
        Err(e) => {
            tracing::warn!(error = %e, "peer's disappearing timer not stored");
            false
        }
    }
}

/// The window displayed `conversation`: the received messages there whose
/// timer had not started start it now, and the expiry task is woken.
pub(crate) async fn displayed(host: &dyn FriendHost, conversation: &str) {
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let (id, now) = (conversation.to_owned(), host.clock().now_ms_i64());
    match state::db_call!(host, "start disappearing timers", move |conn| {
        disappearing::start_on_display(conn, &owner, &id, now)
    })
    .await
    {
        Ok(0) => {}
        Ok(_) => wake(host),
        Err(e) => tracing::warn!(error = %e, "disappearing timers not started"),
    }
}

/// Wake the expiry task: a new `expire_at` may be the earliest.
pub(crate) fn wake(host: &dyn FriendHost) {
    host.stores().expiry.notify_one();
}

/// Delete every message due by now, publish what went, and return the
/// next deadline (ms), if any: one pass of the Host's expiry task.
///
/// # Errors
/// The identity is locked or the storage failed.
pub async fn expire_due(host: &dyn FriendHost) -> Result<Option<i64>, FriendError> {
    let owner = state::owner(host)?;
    let now = host.clock().now_ms_i64();
    let (gone, next) = state::db_call!(host, "delete expired messages", move |conn| {
        let gone = disappearing::delete_expired(conn, &owner, now)?;
        Ok((gone, disappearing::next_expiry(conn, &owner)?))
    })
    .await?;
    for e in gone {
        host.publish(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::MessagesExpired {
                conversation_id: e.conversation_id,
                conversation_type: e.conversation_type,
                client_ids: e.client_ids,
            },
        ));
    }
    Ok(next)
}
