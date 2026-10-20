//! What each opened 1:1 kind does (plan E2.3). An applied message's
//! `unprocessed` row goes once it has run (a DM whose row could not be
//! stored keeps it, so the next unlock's replay applies it, D24); an
//! unported one's goes at its hand-off.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmText};
use rekindle_events::typing::TYPING_TTL_MS;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent, TypingContext};

use super::frame::{processed, Opened};
use super::Routed;
use crate::operations::dm::{client_id_text, NewDmFields};
use crate::operations::friend::inbound::{self, IncomingFriendAccept, IncomingFriendRequest};
use crate::operations::friend::{identity_rotation, session_reset, state, FriendHost};
use crate::operations::{relay, typing};

/// Where a friend's typing to us shows: their DM.
fn dm_typing(sender: &str) -> TypingContext {
    TypingContext::Dm {
        peer_key: sender.to_string(),
    }
}

/// Handle `opened`, or hand it back when its owner step has not come.
pub(super) async fn handle(host: &dyn FriendHost, opened: Opened) -> Routed {
    let sender = opened.sender_hex.as_str();
    let mut applied = true;
    match &opened.body {
        DirectBody::DmText(text) => {
            applied = direct_message(host, sender, text, opened.sent_at_ms).await;
            // Their message ends their typing (plan E2.3).
            typing::stopped(host, &dm_typing(sender), sender);
        }
        DirectBody::Typing(t) => {
            if t.typing {
                typing::started(host, dm_typing(sender), sender, TYPING_TTL_MS);
            } else {
                typing::stopped(host, &dm_typing(sender), sender);
            }
        }
        DirectBody::FriendRequest(r) => {
            let req = IncomingFriendRequest {
                sender_hex: sender,
                display_name: &r.display_name,
                message: &r.message,
                prekey_bundle: &r.prekey_bundle,
                profile_dht_key: &r.profile_dht_key,
                route_blob: &r.route_blob,
                mailbox_dht_key: &r.mailbox_dht_key,
                invite_id: r.invite_id.as_deref(),
            };
            inbound::on_friend_request(host, &req).await;
        }
        DirectBody::FriendAccept(a) => {
            let accept = IncomingFriendAccept {
                sender_hex: sender,
                profile_dht_key: &a.profile_dht_key,
                route_blob: a.route_blob.clone(),
                mailbox_dht_key: &a.mailbox_dht_key,
            };
            inbound::on_friend_accept(host, &accept).await;
        }
        DirectBody::FriendReject => inbound::on_friend_reject(host, sender).await,
        DirectBody::ProfileKeyRotated(p) => {
            inbound::on_profile_key_rotated(host, sender, &p.new_profile_dht_key).await;
        }
        DirectBody::IdentityRotated(r) => {
            identity_rotation::on_identity_rotated(host, sender, r).await;
        }
        DirectBody::Unfriended => inbound::on_unfriended(host, sender).await,
        DirectBody::UnfriendedAck => inbound::on_unfriended_ack(host, sender).await,
        DirectBody::SessionResetRequest(r) => {
            session_reset::on_request(host, sender, &r.prekey_bundle).await;
        }
        DirectBody::SessionResetAccept => session_reset::on_accept(host, sender),
        DirectBody::SessionResetDecline(d) => session_reset::on_decline(host, sender, &d.reason),
        DirectBody::StatusResponse(r) => relay::on_status_response(host, r),
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
        DirectBody::RelayEnvelope(_)
        | DirectBody::GroupDmInvite(_)
        | DirectBody::GroupDmLeave(_)
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
            return Routed::Unported(Box::new(opened));
        }
    }
    if applied {
        processed(host, &opened);
    } else {
        tracing::warn!(from = %sender, kind = ?opened.type_id, "message not applied — kept for the replay");
    }
    Routed::Handled
}

/// Store a friend's direct message (once: a replayed one is already held),
/// count it unread, and publish it; whether it was stored.
async fn direct_message(
    host: &dyn FriendHost,
    sender: &str,
    text: &DmText,
    sent_at_ms: u64,
) -> bool {
    let timestamp = i64::try_from(sent_at_ms).unwrap_or(i64::MAX);
    let message_id = client_id_text(&text.client_id);
    let reply_to = text.reply_to.as_ref().map(client_id_text);
    let fields = NewDmFields {
        peer: sender.to_string(),
        sender: sender.to_string(),
        body: text.body.clone(),
        timestamp,
        is_read: false,
        message_id: message_id.clone(),
        reply_to: reply_to.clone(),
    };
    match fields.store(host).await {
        Ok(stored) if stored.inserted => {}
        // A replay of a message already applied: no second unread or event.
        Ok(_) => return true,
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "incoming message not stored");
            return false;
        }
    }
    state::update(host, sender, |f| f.unread_count += 1);
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
    true
}
