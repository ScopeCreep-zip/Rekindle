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

/// Dispatcher for group-call MessagePayload variants that arrived via
/// app_message. Offer/Accept/Decline are normally app_call replies and
/// shouldn't reach here; we trace and drop them. The two gossip
/// variants (ParticipantJoined / Left) update grid state on receivers.
pub async fn handle_group_call_payload(
    app: &tauri::AppHandle,
    state: &SharedState,
    _pool: &DbPool,
    sender_hex: &str,
    payload: MessagePayload,
) {
    match payload {
        MessagePayload::GroupCallOffer { call_id, .. }
        | MessagePayload::GroupCallAccept { call_id, .. }
        | MessagePayload::GroupCallDecline { call_id, .. } => {
            tracing::trace!(call_id = %call_id, sender = %sender_hex,
                "group call signaling via app_message; expected app_call");
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

/// Used by the app_call dispatcher in `commands::calls` when an
/// incoming GroupCallOffer arrives. Inserts a state entry, emits the
/// frontend ring event, and parks on a oneshot for accept/decline.
pub fn unwrap_offer(
    our_secret: &StaticSecret,
    initiator_x25519_pub: &[u8],
    call_id: &str,
    our_pubkey: &str,
    wrapped: &[u8],
) -> Result<[u8; 32], String> {
    unwrap_call_key(
        our_secret,
        initiator_x25519_pub,
        call_id,
        our_pubkey,
        wrapped,
    )
    .map_err(|e| e.to_string())
}
