//! The 1:1 payload families the Host hands back unported (plan E2.1,
//! `operations::direct::Routed::Unported`), handled by the desktop's
//! existing arms until each owner step deletes its arm: the SMPL DM family
//! and DM video (E2.3, E2.6, E4.3.8), calls and group calls (E4.1), relay
//! offers, envelopes, status requests and push relay (E5.2), and a channel
//! message over a 1:1 envelope (E3.1).

use std::sync::Arc;

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_transport::operations::direct::Prepared;
use tauri::AppHandle;

use crate::channels::ChatEvent;
use crate::db_helpers::db_fire;
use crate::services::message_service::{handle_call_signaling_payload, handle_dm_payload};
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Handle an unported `app_message` payload.
pub(super) async fn handle(app: &AppHandle, state: &Arc<AppState>, pool: &Db, prepared: Prepared) {
    let sender = prepared.sender_hex.as_str();
    match prepared.payload {
        MessagePayload::ChannelMessage {
            channel_id, body, ..
        } => channel_message(state, pool, sender, &channel_id, &body, prepared.timestamp),
        MessagePayload::RelayOffer {
            relay_route_blob,
            relay_pseudonym,
        } => {
            // Bob receives Carol's offer (architecture §13.2 step 2):
            // persist the blob and republish the (padded) pool.
            if let Err(e) = crate::services::relay::add_received_offer(
                state,
                pool,
                &relay_pseudonym,
                &relay_route_blob,
            )
            .await
            {
                tracing::warn!(error = %e, from = %sender, "failed to persist RelayOffer");
            } else {
                crate::services::relay::pool::republish_relay_pool(state, pool).await;
            }
        }
        MessagePayload::RelayWithdraw { relay_pseudonym } => {
            if let Err(e) =
                crate::services::relay::remove_received_offer(state, pool, &relay_pseudonym).await
            {
                tracing::warn!(error = %e, from = %sender, "failed to drop revoked RelayOffer");
            } else {
                crate::services::relay::pool::republish_relay_pool(state, pool).await;
            }
        }
        MessagePayload::RelayEnvelope {
            target_pubkey,
            inner_payload,
        } => {
            // Carol forwards (architecture §13.3 step 3).
            if let Err(e) = crate::services::relay::handle_relay_envelope(
                state,
                pool,
                &target_pubkey,
                &inner_payload,
            )
            .await
            {
                tracing::debug!(error = %e, target = %target_pubkey, "RelayEnvelope dropped");
            }
        }
        payload @ (MessagePayload::DmInvite { .. }
        | MessagePayload::DmAccept { .. }
        | MessagePayload::DmDecline { .. }
        | MessagePayload::GroupDmInvite { .. }
        | MessagePayload::DmLeave { .. }
        | MessagePayload::DmVideoFragment { .. }) => {
            handle_dm_payload(app, state, pool, sender, payload).await;
        }
        payload @ (MessagePayload::CallInvite { .. }
        | MessagePayload::CallAccept { .. }
        | MessagePayload::CallDecline { .. }
        | MessagePayload::CallRinging { .. }
        | MessagePayload::CallEnd { .. }
        | MessagePayload::CallMediaState { .. }
        | MessagePayload::CallReaction { .. }) => {
            handle_call_signaling_payload(app, state, pool, sender, payload).await;
        }
        payload @ (MessagePayload::GroupCallOffer { .. }
        | MessagePayload::GroupCallAccept { .. }
        | MessagePayload::GroupCallDecline { .. }
        | MessagePayload::GroupCallParticipantJoined { .. }
        | MessagePayload::GroupCallParticipantLeft { .. }) => {
            crate::services::calls_adapter::handle_group_call_payload(
                app, state, pool, sender, payload,
            )
            .await;
        }
        MessagePayload::WakeNotify { ts } => {
            crate::services::push_relay::handle_wake_notify(state, ts);
        }
        MessagePayload::StatusRequest { target_pubkey } => {
            // Architecture §13.5 — relay friends serve cached presence, to
            // friends only, for the friends they relay for.
            if let Err(e) = crate::services::relay::respond_to_status_request(
                state,
                pool,
                sender,
                &target_pubkey,
            )
            .await
            {
                tracing::trace!(error = %e, "StatusRequest dropped");
            }
        }
        other => {
            tracing::debug!(from = %sender, kind = ?std::mem::discriminant(&other),
                "a ported payload came back unported — dropped");
        }
    }
}

/// Answer an unported `app_call` payload (the DM invite handshake,
/// architecture §27.1; a relay offer, §13.2 step 2): its reply, or `None`.
pub(super) async fn handle_call(
    app: &AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    prepared: Prepared,
) -> Option<Vec<u8>> {
    let reply = match prepared.payload {
        MessagePayload::RelayOffer {
            relay_route_blob,
            relay_pseudonym,
        } => {
            let ingest = crate::services::relay::add_received_offer(
                state,
                pool,
                &relay_pseudonym,
                &relay_route_blob,
            )
            .await;
            match ingest {
                Ok(()) => {
                    crate::services::relay::pool::republish_relay_pool(state, pool).await;
                    MessagePayload::RelayOfferAck {
                        ok: true,
                        reason: String::new(),
                    }
                }
                Err(e) => MessagePayload::RelayOfferAck {
                    ok: false,
                    reason: e,
                },
            }
        }
        MessagePayload::DmInvite {
            record_key,
            slot_seed,
            alice_pseudonym,
            alice_subkey,
            bob_subkey,
        } => {
            let result = crate::services::dm::handle_incoming_dm_invite(
                app,
                state,
                pool,
                &prepared.sender_hex,
                &record_key,
                &slot_seed,
                &alice_pseudonym,
                alice_subkey,
                bob_subkey,
            )
            .await;
            match result {
                Ok(()) => MessagePayload::DmAccept {
                    record_key: record_key.clone(),
                },
                Err(e) => MessagePayload::DmDecline {
                    record_key: record_key.clone(),
                    reason: e,
                },
            }
        }
        _ => return None,
    };
    Some(serde_json::to_vec(&reply).unwrap_or_else(|_| b"ACK".to_vec()))
}

/// Store a channel message that arrived over a 1:1 envelope and tell the
/// community windows (it names no community).
fn channel_message(
    state: &Arc<AppState>,
    pool: &Db,
    sender_hex: &str,
    channel_id: &str,
    body: &str,
    timestamp: i64,
) {
    let owner_key = state_helpers::owner_key_or_default(state);
    let sender = sender_hex.to_string();
    let ch_id = channel_id.to_string();
    let body_clone = body.to_string();
    db_fire(pool, "persist channel message", move |conn| {
        crate::message_repo::insert_channel_message(
            conn,
            &owner_key,
            &ch_id,
            &sender,
            &body_clone,
            timestamp,
            false,
            None,
        )
    });
    let event = ChatEvent::MessageReceived {
        from: sender_hex.to_string(),
        body: body.to_string(),
        decryption_failed: false,
        automod_blurred: false,
        timestamp: timestamp.cast_unsigned(),
        conversation_id: channel_id.to_string(),
        server_message_id: None,
        reply_to_id: None,
        sender_display_name: None,
    };
    crate::event_dispatch::emit_from_state(
        state,
        crate::event_dispatch::WebviewEvent::ChannelChat {
            community_id: None,
            event,
        },
    );
}
