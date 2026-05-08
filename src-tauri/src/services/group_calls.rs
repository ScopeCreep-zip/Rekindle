//! Wave 12 W12.9 — group call signaling and state.
//!
//! Architecture §10.10 (Chiralgrams) — group calls reuse the existing
//! voice / video transport but a single shared `call_key` is needed
//! across all participants. The initiator generates the key once, then
//! sends each invitee a `GroupCallOffer` carrying that invitee's
//! per-recipient `wrapped_call_key` (sealed via X25519 + HKDF +
//! AES-256-GCM in `rekindle_calls::group::wrap_call_key`).
//!
//! State storage: `state.group_calls: HashMap<call_id, GroupCallState>`
//! parallel to the 1:1 `active_calls`. The 1:1 path stays untouched.

use std::collections::HashSet;

use rekindle_calls::group::unwrap_call_key;
use rekindle_calls::X25519StaticSecret as StaticSecret;
use rekindle_protocol::messaging::envelope::MessagePayload;
use tauri::Emitter;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::state::SharedState;

/// In-memory state for a group call (1:N). Mirrors `rekindle_calls::CallState`
/// but with a participant set instead of a single peer.
pub struct GroupCallState {
    pub call_id: String,
    /// Hex Ed25519 of whoever initiated the call.
    pub initiator_pubkey: String,
    /// 0 = audio, 1 = video. Mirrors `CallKind::as_u8`.
    pub kind: u8,
    /// All invited participants (hex Ed25519). Includes the initiator.
    pub participants: Vec<String>,
    /// Subset that have replied with GroupCallAccept.
    pub accepted: HashSet<String>,
    /// Our X25519 secret. Set on offer creation (we're the initiator)
    /// or on offer receipt (we're an invitee). Drops zeroize.
    pub our_x25519_secret: Option<StaticSecret>,
    /// 32-byte shared call_key once established. Drops zeroize.
    pub call_key: Option<[u8; 32]>,
    /// Lifecycle marker. `Outgoing` = initiator awaiting accepts;
    /// `Incoming` = we received the offer and need user accept;
    /// `Active` = at least one accept; `Ended` = caller hung up or
    /// last participant left.
    pub status: GroupCallStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupCallStatus {
    Outgoing,
    Incoming,
    Active,
    Ended,
}

/// Drop impl is implicit via StaticSecret + array; both zeroize.

/// Wave 13 W13.13 — dispatcher for group-call MessagePayload variants
/// that arrived via app_message. After W13.13, ALL group call
/// signaling travels via app_message (was app_call in W12.9; the
/// architectural mismatch was the same one that broke 1:1 calls).
pub async fn handle_group_call_payload(
    app: &tauri::AppHandle,
    state: &SharedState,
    _pool: &DbPool,
    sender_hex: &str,
    payload: MessagePayload,
) {
    match payload {
        MessagePayload::GroupCallOffer {
            call_id,
            offer_kind,
            initiator_pubkey,
            initiator_x25519_pub,
            participants,
            wrapped_call_key,
            expires_at_ms,
        } => {
            handle_incoming_group_invite(
                app,
                state,
                sender_hex,
                &call_id,
                offer_kind,
                &initiator_pubkey,
                &initiator_x25519_pub,
                participants,
                &wrapped_call_key,
                expires_at_ms,
            )
            .await;
        }
        MessagePayload::GroupCallAccept {
            call_id,
            acceptor_pubkey,
        } => {
            handle_group_accept_received(app, state, sender_hex, &call_id, &acceptor_pubkey).await;
        }
        MessagePayload::GroupCallDecline { call_id, reason } => {
            handle_group_decline_received(app, state, sender_hex, &call_id, reason).await;
        }
        MessagePayload::GroupCallParticipantJoined { call_id, participant_pubkey } => {
            // Receivers update their grid only if the announced
            // participant is in the call's known invite list — defends
            // against a malicious peer announcing arbitrary joins.
            let known = {
                let calls = state.group_calls.lock();
                calls
                    .get(&call_id)
                    .map(|c| c.participants.contains(&participant_pubkey))
                    .unwrap_or(false)
            };
            if known {
                let _ = app.emit(
                    "chat-event",
                    &ChatEvent::GroupCallParticipantJoined {
                        call_id,
                        participant_pubkey,
                    },
                );
            }
        }
        MessagePayload::GroupCallParticipantLeft {
            call_id,
            participant_pubkey,
            reason,
        } => {
            let known = {
                let calls = state.group_calls.lock();
                calls
                    .get(&call_id)
                    .map(|c| c.participants.contains(&participant_pubkey))
                    .unwrap_or(false)
            };
            if known {
                let _ = app.emit(
                    "chat-event",
                    &ChatEvent::GroupCallParticipantLeft {
                        call_id,
                        participant_pubkey,
                        reason,
                    },
                );
            }
        }
        _ => unreachable!("group_calls dispatcher called with non-group payload"),
    }
}

/// Wave 13 — receiver-side entry into a group call. Mirrors the
/// `handle_incoming_invite` shape for 1:1 but with the per-recipient
/// X25519 unwrap.
pub async fn handle_incoming_group_invite(
    app: &tauri::AppHandle,
    state: &SharedState,
    sender_hex: &str,
    call_id: &str,
    offer_kind: u8,
    initiator_pubkey: &str,
    initiator_x25519_pub: &[u8],
    participants: Vec<String>,
    wrapped_call_key: &[u8],
    expires_at_ms: u64,
) {
    use rekindle_calls::CallKind;
    let kind = CallKind::from_u8(offer_kind).unwrap_or(CallKind::Audio);
    let display_name = crate::state_helpers::friend_display_name(state, sender_hex)
        .unwrap_or_else(|| {
            if initiator_pubkey.len() > 16 {
                format!("{}…", &initiator_pubkey[..16])
            } else {
                initiator_pubkey.to_string()
            }
        });

    if initiator_x25519_pub.len() != 32 {
        tracing::warn!(call_id = %call_id, "GroupCallOffer with bad x25519 length");
        return;
    }

    // Temp-mute lookup mirrors 1:1 — silently drop if peer is muted.
    {
        let now = rekindle_utils::timestamp_ms();
        let mut muted = state.temp_call_muted.lock();
        if let Some(&expires_at) = muted.get(sender_hex) {
            if now < expires_at {
                return;
            }
            muted.remove(sender_hex);
        }
    }

    // Convert our Ed25519 secret → X25519 (same as W12.9 receiver path).
    let our_ed_pubkey = match crate::state_helpers::current_identity(state) {
        Ok(i) => i.public_key,
        Err(_) => return,
    };
    let our_x25519_secret = {
        let secret_opt = state.identity_secret.lock().clone();
        match secret_opt {
            Some(bytes) => {
                let identity = rekindle_crypto::Identity::from_secret_bytes(&bytes);
                identity.to_x25519_secret()
            }
            None => return,
        }
    };
    let call_key = match unwrap_call_key(
        &our_x25519_secret,
        initiator_x25519_pub,
        call_id,
        &our_ed_pubkey,
        wrapped_call_key,
    ) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!(call_id = %call_id, error = %e,
                "group call key unwrap failed (likely not a participant)");
            return;
        }
    };

    {
        let mut calls = state.group_calls.lock();
        calls.insert(
            call_id.to_string(),
            GroupCallState {
                call_id: call_id.to_string(),
                initiator_pubkey: initiator_pubkey.to_string(),
                kind: offer_kind,
                participants: participants.clone(),
                accepted: HashSet::new(),
                our_x25519_secret: Some(our_x25519_secret),
                call_key: Some(call_key),
                status: GroupCallStatus::Incoming,
            },
        );
    }

    let kind_str = match kind {
        CallKind::Audio => "audio",
        CallKind::Video => "video",
    };
    let _ = app.emit(
        "chat-event",
        &ChatEvent::IncomingGroupCall {
            call_id: call_id.to_string(),
            from: sender_hex.to_string(),
            display_name,
            kind: kind_str.into(),
            participants,
            expires_at_ms,
        },
    );
    crate::windows::surface_window_for_call(app);
}

/// Wave 13 — initiator receives a participant's accept. Updates the
/// GroupCallState (Outgoing → Active on first accept), emits
/// ChatEvent::GroupCallConnected on first transition + always emits
/// ParticipantJoined so the grid updates.
pub async fn handle_group_accept_received(
    app: &tauri::AppHandle,
    state: &SharedState,
    sender_hex: &str,
    call_id: &str,
    acceptor_pubkey: &str,
) {
    let _ = sender_hex; // sender == acceptor in normal flow
    let became_active = {
        let mut calls = state.group_calls.lock();
        let Some(call) = calls.get_mut(call_id) else {
            return;
        };
        if !call.participants.contains(&acceptor_pubkey.to_string()) {
            tracing::warn!(call = %call_id, acceptor = %acceptor_pubkey,
                "GroupCallAccept from non-invitee; ignoring");
            return;
        }
        call.accepted.insert(acceptor_pubkey.to_string());
        if call.status == GroupCallStatus::Outgoing {
            call.status = GroupCallStatus::Active;
            true
        } else {
            false
        }
    };
    if became_active {
        let _ = app.emit(
            "chat-event",
            &ChatEvent::GroupCallConnected {
                call_id: call_id.to_string(),
            },
        );
    }
    let _ = app.emit(
        "chat-event",
        &ChatEvent::GroupCallParticipantJoined {
            call_id: call_id.to_string(),
            participant_pubkey: acceptor_pubkey.to_string(),
        },
    );
}

/// Wave 13 — participant declines a group call.
pub async fn handle_group_decline_received(
    app: &tauri::AppHandle,
    state: &SharedState,
    sender_hex: &str,
    call_id: &str,
    reason: String,
) {
    let known = {
        let calls = state.group_calls.lock();
        calls.contains_key(call_id)
    };
    if !known {
        return;
    }
    let _ = app.emit(
        "chat-event",
        &ChatEvent::GroupCallParticipantLeft {
            call_id: call_id.to_string(),
            participant_pubkey: sender_hex.to_string(),
            reason,
        },
    );
}

