//! What each opened 1:1 kind does (plan E2.3). An applied message's
//! `unprocessed` row goes once it has run; every kind that stores reports
//! whether its store landed, and one whose store failed keeps its row, so
//! the next unlock's replay applies it (D24; audit G5). An unported one's
//! row goes at its hand-off.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmText, DmTypeId};
use rekindle_events::typing::TYPING_TTL_MS;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent, TypingContext};

use super::frame::{processed, Opened};
use super::Routed;
use crate::operations::dm::{client_id_text, NewDmFields};
use crate::operations::friend::inbound::{self, IncomingFriendAccept, IncomingFriendRequest};
use crate::operations::friend::{
    bio, game_invite, game_totals, identity_rotation, presence_key, session_reset, state,
    FriendHost,
};
use crate::operations::{relay, typing};

/// Where a friend's typing to us shows: their DM.
fn dm_typing(sender: &str) -> TypingContext {
    TypingContext::Dm {
        peer_key: sender.to_string(),
    }
}

/// What one apply came to.
pub(super) enum Applied {
    /// It ran; `false` when its store failed (its row is kept for the
    /// replay, a sealed frame's nonce forgotten for its retry).
    Done(bool),
    /// Judged and refused (a policy, not a failure): its row goes and its
    /// nonce stays seen, so a replay is a duplicate (RV5 R4).
    Refused,
    /// A kind whose owner step has not come, handed back.
    Unported(Box<Opened>),
}

/// Handle `opened`, or hand it back when its owner step has not come.
pub(super) async fn handle(host: &dyn FriendHost, opened: Opened) -> Routed {
    match apply(host, opened).await {
        Applied::Done(_) | Applied::Refused => Routed::Handled,
        Applied::Unported(opened) => Routed::Unported(opened),
    }
}

/// Apply `opened`, and say whether it stored.
pub(super) async fn apply(host: &dyn FriendHost, opened: Opened) -> Applied {
    let sender = opened.sender_hex.as_str();
    let mut applied = true;
    match &opened.body {
        DirectBody::DmText(text) => {
            // A hint of theirs arrived: they are reachable (E2.5g).
            crate::operations::dm::outbox::peer_heard(host, sender).await;
            applied = direct_message(host, sender, text, &opened).await;
            if applied {
                // Committed: acknowledge it (a resend of one held again too,
                // its first ack may be lost).
                crate::operations::dm::ack::committed(
                    host,
                    sender,
                    &opened.body,
                    opened.stream_seq,
                    opened.chain_seq,
                )
                .await;
            }
            // Their message ends their typing (plan E2.3).
            typing::stopped(host, &dm_typing(sender), sender);
        }
        DirectBody::DmAck(a) => crate::operations::dm::ack::on_ack(host, sender, a).await,
        // Ignored, never stored, while our own setting is off (plan E2.5).
        DirectBody::ReadUpTo(r) => {
            crate::operations::dm::receipts::on_read_up_to(host, sender, r).await;
        }
        DirectBody::Typing(t) => {
            if t.typing {
                let name = typing::peer_name(host, sender);
                typing::started(host, dm_typing(sender), sender, name, TYPING_TTL_MS);
            } else {
                typing::stopped(host, &dm_typing(sender), sender);
            }
        }
        DirectBody::FriendRequest(r) => {
            let req = IncomingFriendRequest {
                sender_hex: sender,
                display_name: &r.display_name,
                message: &r.message,
                profile_dht_key: &r.profile_dht_key,
                route_blob: &r.route_blob,
                mailbox_dht_key: &r.mailbox_dht_key,
                auth: &r.auth,
                reply: &r.reply,
            };
            match inbound::on_friend_request(host, &req).await {
                Ok(()) => {}
                // The request's own values failed the store: judged
                // (review RV5 M2).
                Err(e) if e.is_input() => {
                    tracing::info!(from = %sender, error = %e, "friend request refused by its store");
                    processed(host, &opened);
                    return Applied::Refused;
                }
                Err(_) => applied = false,
            }
        }
        DirectBody::FriendAccept(a) => {
            let accept = IncomingFriendAccept {
                sender_hex: sender,
                profile_dht_key: &a.profile_dht_key,
                route_blob: a.route_blob.clone(),
                mailbox_dht_key: &a.mailbox_dht_key,
            };
            applied = inbound::on_friend_accept(host, &accept).await;
        }
        DirectBody::ProfileKeyRotated(p) => {
            applied = inbound::on_profile_key_rotated(host, sender, &p.new_profile_dht_key).await;
        }
        DirectBody::FriendsKey(k) => {
            applied = presence_key::on_friends_key(host, sender, k).await;
        }
        DirectBody::ProfileBio(b) => {
            applied = bio::on_profile_bio(host, sender, b).await;
        }
        DirectBody::GameInvite(i) => {
            applied = game_invite::on_game_invite(host, sender, i);
        }
        DirectBody::GameTotals(t) => {
            applied = game_totals::on_game_totals(host, sender, t).await;
        }
        // A group's timer is checked against the group it names; a DM's
        // needs the friend gate (plan E7.1).
        DirectBody::DisappearingTimer(t) if t.group_id.is_some() => {
            applied = crate::operations::dm::disappearing::on_timer(host, sender, t).await;
        }
        DirectBody::DisappearingTimer(t) => {
            if !super::admission::friend_admitted(host, sender, DmTypeId::DisappearingTimer).await {
                processed(host, &opened);
                return Applied::Refused;
            }
            applied = crate::operations::dm::disappearing::on_timer(host, sender, t).await;
        }
        DirectBody::StreamRecord(r) if r.group_id.is_some() => {
            applied = crate::operations::group_dm::on_stream_record(host, sender, r).await;
        }
        DirectBody::StreamRecord(r) => {
            // A group member who is not a friend passed the gate for a
            // group's record only: a 1:1 one needs the friend gate.
            if !super::admission::friend_admitted(host, sender, DmTypeId::StreamRecord).await {
                processed(host, &opened);
                return Applied::Refused;
            }
            applied = crate::operations::dm::stream::on_stream_record(host, sender, r).await;
            if applied {
                // Stored (or refused from a non-friend, which `committed`
                // never acknowledges): its row settles on our `DmAck`.
                crate::operations::dm::ack::committed(
                    host,
                    sender,
                    &opened.body,
                    opened.stream_seq,
                    opened.chain_seq,
                )
                .await;
            }
        }
        DirectBody::IdentityRotated(r) => {
            applied = identity_rotation::on_identity_rotated(host, sender, r).await;
        }
        DirectBody::Unfriended => applied = inbound::on_unfriended(host, sender).await,
        DirectBody::UnfriendedAck => applied = inbound::on_unfriended_ack(host, sender).await,
        DirectBody::SessionResetRequest(r) => {
            applied = session_reset::on_request(host, sender, &r.reason).await;
        }
        DirectBody::RetryRequest(r) => {
            applied = super::retry_requests::on_retry_request(host, sender, r).await;
        }
        DirectBody::SessionResetAccept => session_reset::on_accept(host, sender),
        DirectBody::PreKeyRejected(r) => {
            match crate::operations::prekeys::on_rejected(host, sender, r).await {
                crate::operations::prekeys::Rejection::Applied => {}
                crate::operations::prekeys::Rejection::NotOurs => {
                    processed(host, &opened);
                    return Applied::Refused;
                }
                crate::operations::prekeys::Rejection::Failed => applied = false,
            }
        }
        DirectBody::SessionResetDecline(_) => session_reset::on_decline(host, sender),
        DirectBody::StatusResponse(r) => relay::on_status_response(host, sender, r),
        DirectBody::RelayOfferAck(_) => {
            tracing::trace!("received RelayOfferAck via app_message; ignoring");
        }
        // The push relay is its own binary; a client only sends these.
        DirectBody::RegisterPushRelay(_) | DirectBody::UnregisterPushRelay(_) => {
            tracing::debug!("ignoring push-relay registration at a client");
        }
        // Defined here for E6's DM parity; E6.4 applies them with their
        // rules (the author only, 48 h, ten edits, delete wins).
        DirectBody::DmEdit(_) | DirectBody::DmDelete(_) | DirectBody::DmReaction(_) => {
            tracing::debug!(from = %sender, kind = ?opened.type_id, "DM edit/delete/reaction not applied before E6.4");
        }
        // Group DMs (plan E2.6) are `rekindle-dm`'s, which applies them from
        // slice G2 with the roster admission; until then they are dropped
        // here, never handed to the desktop's retired SMPL group path.
        DirectBody::GroupDmInvite(_)
        | DirectBody::GroupDmAccept(_)
        | DirectBody::GroupDmDecline(_)
        | DirectBody::GroupDmLeave(_)
        | DirectBody::GroupDmRosterGrow(_)
        | DirectBody::GroupContent(_) => {
            applied = crate::operations::group_dm::apply(host, sender, &opened.body).await;
        }
        // Only ever sealed inside a group entry (plan E7.2 G7).
        DirectBody::GroupDmRename(_) => {
            tracing::info!(from = %sender, "GroupDmRename outside a group entry; refused");
            applied = false;
        }
        // Only ever sealed inside a group entry (plan E7.2 G6).
        DirectBody::LobbyOpen(_)
        | DirectBody::LobbyJoin(_)
        | DirectBody::LobbyLeave(_)
        | DirectBody::LobbyLaunch(_)
        | DirectBody::LobbyClose(_) => {
            tracing::info!(from = %sender, kind = ?opened.type_id, "lobby kind outside a group entry; refused");
            applied = false;
        }
        DirectBody::CreatorInvite(_)
        | DirectBody::CreatorAccept(_)
        | DirectBody::CreatorDecline(_)
        | DirectBody::CreatorWithdrawn(_)
        | DirectBody::CreatorJoin(_) => {
            applied =
                crate::operations::friend::creator_round::on_frame(host, sender, &opened.body)
                    .await;
        }
        DirectBody::RelayEnvelope(_)
        | DirectBody::RelayOffer(_)
        | DirectBody::RelayWithdraw(_)
        | DirectBody::StatusRequest(_)
        | DirectBody::WakeNotify(_)
        | DirectBody::DmVideoFragment(_)
        | DirectBody::CallInvite(_)
        | DirectBody::CallRinging(_)
        | DirectBody::CallAccept(_)
        | DirectBody::CallDecline(_)
        | DirectBody::CallEnd(_)
        | DirectBody::CallMediaState(_)
        | DirectBody::CallReaction(_)
        | DirectBody::GroupCallOffer(_)
        | DirectBody::GroupCallAccept(_)
        | DirectBody::GroupCallDecline(_)
        | DirectBody::GroupCallParticipantJoined(_)
        | DirectBody::GroupCallParticipantLeft(_) => {
            processed(host, &opened);
            return Applied::Unported(Box::new(opened));
        }
    }
    if applied {
        processed(host, &opened);
    } else {
        tracing::warn!(from = %sender, kind = ?opened.type_id, "message not applied — kept for the replay");
        if let (Some(nonce), Ok(from)) = (opened.nonce, hex::decode(sender)) {
            if let Ok(from) = <[u8; 32]>::try_from(from) {
                super::frame::forget(host, &from, &nonce);
            }
        }
    }
    Applied::Done(applied)
}

/// Store a friend's direct message (once: a replayed one is already held),
/// count it unread, and publish it; whether it was stored.
async fn direct_message(
    host: &dyn FriendHost,
    sender: &str,
    text: &DmText,
    opened: &Opened,
) -> bool {
    let stream_seq = opened.stream_seq;
    let claimed = i64::try_from(opened.sent_at_ms).unwrap_or(i64::MAX);
    let message_id = client_id_text(&text.client_id);
    let reply_to = text.reply_to.as_ref().map(client_id_text);
    let fields = NewDmFields {
        peer: sender.to_string(),
        sender: sender.to_string(),
        body: text.body.clone(),
        timestamp: claimed,
        is_read: false,
        message_id: message_id.clone(),
        reply_to: reply_to.clone(),
        send_state: rekindle_types::message::SendState::Delivered,
        stream_seq: i64::try_from(stream_seq).ok().filter(|s| *s >= 1),
        expire_seconds: text.expire_seconds,
    };
    let timestamp = match fields.store_received(host, opened.unprocessed).await {
        Ok((stored, at)) if stored.inserted => at,
        // A replay of a message already applied: no second unread or event.
        Ok(_) => return true,
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "incoming message not stored");
            return false;
        }
    };
    // So a run's logs time each arrival (A/B finding R5-5); the peer key
    // is redacted to its per-process tag by the log scrubber.
    tracing::info!(
        from = %sender,
        stream_seq,
        chain_seq = opened.chain_seq,
        "direct message received"
    );
    host.publish(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DirectMessageReceived {
            peer_key: sender.to_string(),
            body: Some(text.body.clone()),
            decryption_failed: false,
            automod_blurred: false,
            timestamp: timestamp.cast_unsigned(),
            conversation_id: sender.to_string(),
            sender_name: state::friend(host, sender).map(|f| f.display_name),
            server_message_id: Some(message_id),
            reply_to_id: reply_to,
            is_own: false,
        },
    ));
    crate::operations::dm::publish_dm_unread(host, sender).await;
    true
}
