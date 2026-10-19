//! What each opened 1:1 payload does.

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_types::subscription_events::{
    ChannelMessageEvent, FriendEvent, SubscriptionEvent, TypingContext, TypingEvent,
};

use super::{Prepared, Routed};
use crate::operations::friend::inbound::{self, IncomingFriendAccept, IncomingFriendRequest};
use crate::operations::friend::session_reset::{self, ResetAccept};
use crate::operations::friend::{state, FriendHost};
use crate::operations::relay;

/// Handle `prepared`, or hand it back when its owner step has not come.
pub(super) async fn handle(host: &dyn FriendHost, prepared: Prepared) -> Routed {
    let sender = prepared.sender_hex.as_str();
    match &prepared.payload {
        MessagePayload::DirectMessage { body, .. } => {
            direct_message(host, sender, body, prepared.timestamp);
        }
        MessagePayload::TypingIndicator { typing } => {
            let context = TypingContext::Dm {
                peer_key: sender.to_string(),
            };
            let who = sender.to_string();
            host.publish(SubscriptionEvent::Typing(if *typing {
                TypingEvent::Started { context, who }
            } else {
                TypingEvent::Stopped { context, who }
            }));
        }
        MessagePayload::FriendRequest {
            display_name,
            message,
            prekey_bundle,
            profile_dht_key,
            route_blob,
            mailbox_dht_key,
            invite_id,
        } => {
            let req = IncomingFriendRequest {
                sender_hex: sender,
                display_name,
                message,
                prekey_bundle,
                profile_dht_key,
                route_blob,
                mailbox_dht_key,
                invite_id: invite_id.as_deref(),
            };
            inbound::on_friend_request(host, &req).await;
        }
        MessagePayload::FriendAccept {
            prekey_bundle,
            profile_dht_key,
            route_blob,
            mailbox_dht_key,
            ephemeral_key,
            signed_prekey_id,
            one_time_prekey_id,
            ml_kem_ciphertext,
            used_ot_pqpk_id,
        } => {
            let accept = IncomingFriendAccept {
                sender_hex: sender,
                prekey_bundle,
                profile_dht_key,
                route_blob: route_blob.clone(),
                mailbox_dht_key,
                ephemeral_key,
                signed_prekey_id: *signed_prekey_id,
                one_time_prekey_id: *one_time_prekey_id,
                ml_kem_ciphertext,
                used_ot_pqpk_id: *used_ot_pqpk_id,
            };
            inbound::on_friend_accept(host, &accept).await;
        }
        MessagePayload::FriendReject => inbound::on_friend_reject(host, sender),
        MessagePayload::FriendRequestReceived => {
            host.publish(SubscriptionEvent::Friend(
                FriendEvent::RequestAcknowledged {
                    peer_key: sender.to_string(),
                },
            ));
        }
        MessagePayload::ProfileKeyRotated {
            new_profile_dht_key,
        } => inbound::on_profile_key_rotated(host, sender, new_profile_dht_key).await,
        MessagePayload::Unfriended => inbound::on_unfriended(host, sender).await,
        MessagePayload::UnfriendedAck => inbound::on_unfriended_ack(host, sender),
        MessagePayload::SessionResetRequest { our_prekey_bundle } => {
            session_reset::on_request(host, sender, our_prekey_bundle).await;
        }
        MessagePayload::SessionResetAccept {
            ephemeral_key,
            signed_prekey_id,
            one_time_prekey_id,
            our_identity_key,
            ml_kem_ciphertext,
            used_ot_pqpk_id,
        } => session_reset::on_accept(
            host,
            sender,
            &ResetAccept {
                ephemeral_key,
                signed_prekey_id: *signed_prekey_id,
                one_time_prekey_id: *one_time_prekey_id,
                identity_key: our_identity_key,
                ml_kem_ciphertext,
                used_ot_pqpk_id: *used_ot_pqpk_id,
            },
        ),
        MessagePayload::SessionResetDecline { reason } => {
            session_reset::on_decline(host, sender, reason);
        }
        MessagePayload::StatusResponse {
            target_pubkey,
            status,
            status_message,
            last_seen,
            route_blob,
        } => relay::on_status_response(
            host,
            &rekindle_codec::capnp_codec::direct::StatusResponse {
                target_pubkey: target_pubkey.clone(),
                status: status.clone(),
                status_message: status_message.clone(),
                last_seen: *last_seen,
                route_blob: route_blob.clone(),
            },
        ),
        // Presence travels in the profile record, not in 1:1 messages.
        MessagePayload::PresenceUpdate { .. } => {}
        MessagePayload::RelayOfferAck { .. } => {
            tracing::trace!("received RelayOfferAck via app_message; ignoring");
        }
        // The push relay is its own binary; a client only sends these.
        MessagePayload::RegisterPushRelay { .. } | MessagePayload::UnregisterPushRelay { .. } => {
            tracing::debug!("ignoring push-relay register payload at a client");
        }
        MessagePayload::ChannelMessage { .. }
        | MessagePayload::RelayOffer { .. }
        | MessagePayload::RelayWithdraw { .. }
        | MessagePayload::RelayEnvelope { .. }
        | MessagePayload::DmInvite { .. }
        | MessagePayload::DmAccept { .. }
        | MessagePayload::DmDecline { .. }
        | MessagePayload::GroupDmInvite { .. }
        | MessagePayload::DmLeave { .. }
        | MessagePayload::DmVideoFragment { .. }
        | MessagePayload::WakeNotify { .. }
        | MessagePayload::StatusRequest { .. }
        | MessagePayload::CallInvite { .. }
        | MessagePayload::CallRinging { .. }
        | MessagePayload::CallAccept { .. }
        | MessagePayload::CallDecline { .. }
        | MessagePayload::CallEnd { .. }
        | MessagePayload::CallMediaState { .. }
        | MessagePayload::CallReaction { .. }
        | MessagePayload::GroupCallOffer { .. }
        | MessagePayload::GroupCallAccept { .. }
        | MessagePayload::GroupCallDecline { .. }
        | MessagePayload::GroupCallParticipantJoined { .. }
        | MessagePayload::GroupCallParticipantLeft { .. } => {
            return Routed::Unported(Box::new(prepared))
        }
    }
    Routed::Handled
}

/// Store a friend's direct message, count it unread, and publish it.
fn direct_message(host: &dyn FriendHost, sender: &str, body: &str, timestamp: i64) {
    if let Ok(owner) = state::owner(host) {
        let (peer, text) = (sender.to_string(), body.to_string());
        state::db_fire!(host, "persist incoming message", move |conn| {
            rekindle_db::repo::messages::insert_dm(
                conn, &owner, &peer, &peer, &text, timestamp, false,
            )
            .map(|_| ())
        });
    }
    state::update(host, sender, |f| f.unread_count += 1);
    host.publish(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DirectMessageReceived {
            peer_key: sender.to_string(),
            body: Some(body.to_string()),
            decryption_failed: false,
            automod_blurred: false,
            timestamp: timestamp.cast_unsigned(),
            conversation_id: sender.to_string(),
            server_message_id: None,
            reply_to_id: None,
            sender_name: None,
        },
    ));
}
