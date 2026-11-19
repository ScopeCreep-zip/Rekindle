//! Disappearing messages (plan E7.1), Signal's mode: each DM and group DM
//! has one timer, set by any member with a `DisappearingTimer` (0x0160)
//! that every member applies and shows as a timeline notice; each message
//! carries the timer in force when it was sent (`DmText.expire_seconds`),
//! so a change racing a message cannot leave two copies on different
//! timers. Our own copy expires at send + timer, a received one at first
//! display + timer. The Host's expiry task ([`expire_due`]) deletes what
//! is due and publishes `MessagesExpired`.

use rekindle_codec::capnp_codec::direct::{DirectBody, DisappearingTimer};
use rekindle_db::repo::disappearing::{self, TimerChange, TimerChanged};
use rekindle_types::disappearing::valid;
use rekindle_types::dto::dm::DisappearingTimerView;
use rekindle_types::message::ConversationType;
use rekindle_types::subscription_events::{
    ChannelMessageEvent, ConversationContext, SubscriptionEvent,
};

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

/// The timer `conversation` (a friend's key, or a group's id) carries now,
/// with the presets and bound the chat window's control offers.
///
/// # Errors
/// Not unlocked, or the storage failed.
pub async fn current(
    host: &dyn FriendHost,
    conversation: &str,
    kind: ConversationType,
) -> Result<DisappearingTimerView, FriendError> {
    let owner = state::owner(host)?;
    let id = conversation.to_owned();
    let seconds = state::db_call!(host, "read the disappearing timer", move |conn| {
        disappearing::timer(conn, &owner, &id, kind)
    })
    .await?;
    Ok(DisappearingTimerView::of(seconds))
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
    match store_change(host, &owner, conversation, kind, seconds, &owner).await? {
        TimerChanged::NotHeld => {
            return Err(FriendError::NotFound(format!(
                "no conversation {conversation}"
            )));
        }
        // Signal sends no update for an unchanged timer.
        TimerChanged::Unchanged => return Ok(()),
        TimerChanged::Changed => {}
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
    first_error.map_or(Ok(()), Err)
}

/// Store `set_by`'s change of the timer on `conversation` and its notice
/// row (one transaction), then, when it changed, publish the notice and
/// wake the expiry task (our notice's timer runs from now).
async fn store_change(
    host: &dyn FriendHost,
    owner: &str,
    conversation: &str,
    kind: ConversationType,
    seconds: u32,
    set_by: &str,
) -> Result<TimerChanged, FriendError> {
    let notice_id = client_id_text(&mint_client_id(host));
    let now = host.clock().now_ms_i64();
    let (o, id, by, nid) = (
        owner.to_owned(),
        conversation.to_owned(),
        set_by.to_owned(),
        notice_id.clone(),
    );
    let outcome = state::db_call!(host, "store the disappearing timer", move |conn| {
        disappearing::change_timer(
            conn,
            &TimerChange {
                owner_key: &o,
                conversation: &id,
                kind,
                seconds,
                set_by: &by,
                now,
                notice_id: &nid,
            },
        )
    })
    .await?;
    if outcome == TimerChanged::Changed {
        if let Some(context) = ConversationContext::of(conversation, kind) {
            host.publish(SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::DisappearingTimerChanged {
                    context,
                    seconds,
                    set_by: set_by.to_owned(),
                    client_id: notice_id,
                    timestamp: now,
                },
            ));
        }
        if seconds > 0 && set_by == owner {
            wake(host);
        }
    }
    Ok(outcome)
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
    match store_change(host, &owner, &conversation, kind, t.seconds, sender).await {
        Ok(_) => true,
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
        let Some(context) = ConversationContext::of(&e.conversation_id, e.conversation_type) else {
            continue;
        };
        host.publish(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::MessagesExpired {
                context,
                client_ids: e.client_ids,
            },
        ));
    }
    Ok(next)
}
