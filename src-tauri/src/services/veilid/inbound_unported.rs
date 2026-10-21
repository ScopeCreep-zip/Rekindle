//! The 1:1 kinds the Host hands back unported (plan E2.3,
//! `operations::direct::Routed::Unported`), handled by the desktop's
//! existing arms until each owner step deletes its arm: the SMPL group DM
//! kinds and DM video (E2.6, E4.3.8), calls and group calls (E4.1), and
//! relay offers, envelopes, status requests and push relay (E5.2).

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::{
    DirectBody, RelayEnvelope, RelayOffer, RelayOfferAck, RelayWithdraw, StatusRequest, WakeNotify,
};
use rekindle_transport::operations::direct::Opened;
use rekindle_transport::operations::dm;
use tauri::AppHandle;

use crate::services::message_service::{handle_call_signaling_payload, handle_dm_payload};
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Handle an unported `app_message` kind.
pub(super) async fn handle(app: &AppHandle, state: &Arc<AppState>, pool: &Db, opened: Opened) {
    let sender = opened.sender_hex.as_str();
    match opened.body {
        DirectBody::RelayOffer(offer) => {
            // Bob receives Carol's offer (architecture §13.2 step 2):
            // persist the blob and republish the (padded) pool.
            if let Err(e) = accept_offer(state, pool, &offer).await {
                tracing::warn!(error = %e, from = %sender, "failed to persist RelayOffer");
            }
        }
        DirectBody::RelayWithdraw(RelayWithdraw { relay_pseudonym }) => {
            if let Err(e) =
                crate::services::relay::remove_received_offer(state, pool, &relay_pseudonym).await
            {
                tracing::warn!(error = %e, from = %sender, "failed to drop revoked RelayOffer");
            } else {
                crate::services::relay::pool::republish_relay_pool(state, pool).await;
            }
        }
        DirectBody::RelayEnvelope(RelayEnvelope {
            target_pubkey,
            inner_payload,
        }) => {
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
        body @ DirectBody::DmVideoFragment(_) => {
            handle_dm_payload(state, sender, body);
        }
        body @ (DirectBody::CallInvite(_)
        | DirectBody::CallAccept(_)
        | DirectBody::CallDecline(_)
        | DirectBody::CallRinging(_)
        | DirectBody::CallEnd(_)
        | DirectBody::CallMediaState(_)
        | DirectBody::CallReaction(_)) => {
            handle_call_signaling_payload(app, state, pool, sender, body).await;
        }
        body @ (DirectBody::GroupCallOffer(_)
        | DirectBody::GroupCallAccept(_)
        | DirectBody::GroupCallDecline(_)
        | DirectBody::GroupCallParticipantJoined(_)
        | DirectBody::GroupCallParticipantLeft(_)) => {
            crate::services::calls_adapter::handle_group_call_payload(
                app, state, pool, sender, body,
            )
            .await;
        }
        DirectBody::WakeNotify(WakeNotify { ts }) => {
            crate::services::push_relay::handle_wake_notify(state, ts);
        }
        DirectBody::StatusRequest(StatusRequest { target_pubkey }) => {
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
            tracing::debug!(from = %sender, kind = ?other.type_id(),
                "a ported kind came back unported — dropped");
        }
    }
}

/// Answer an unported `app_call` (a relay offer, architecture §13.2 step
/// 2): a `RelayOfferAck` frame under our session with the caller, or an
/// empty reply when none can be sealed.
pub(super) async fn handle_call(state: &Arc<AppState>, pool: &Db, opened: Opened) -> Vec<u8> {
    let DirectBody::RelayOffer(offer) = &opened.body else {
        tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
            "unported app_call kind — empty reply");
        return Vec::new();
    };
    let ack = match accept_offer(state, pool, offer).await {
        Ok(()) => RelayOfferAck {
            ok: true,
            reason: String::new(),
        },
        Err(reason) => RelayOfferAck { ok: false, reason },
    };
    match seal_reply(state, &opened.sender_hex, DirectBody::RelayOfferAck(ack)).await {
        Ok(reply) => reply,
        Err(e) => {
            tracing::warn!(to = %opened.sender_hex, error = %e, "RelayOfferAck not sealed");
            Vec::new()
        }
    }
}

/// Persist a friend's relay offer and republish the pool.
async fn accept_offer(state: &Arc<AppState>, pool: &Db, offer: &RelayOffer) -> Result<(), String> {
    crate::services::relay::add_received_offer(
        state,
        pool,
        &offer.relay_pseudonym,
        &offer.relay_route_blob,
    )
    .await?;
    crate::services::relay::pool::republish_relay_pool(state, pool).await;
    Ok(())
}

/// `body` to `to` as the transport frames an `app_call` reply.
async fn seal_reply(state: &AppState, to: &str, body: DirectBody) -> Result<Vec<u8>, String> {
    let host = state_helpers::friend_host(state)?;
    dm::reply_frame(host.as_ref(), to, &body)
        .await
        .map_err(|e| e.to_string())
}
