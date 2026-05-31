//! DM (direct message) payload types.
//!
//! These are the inner payloads carried inside a [`SignedPayload`] envelope.
//! For session-based types (DirectMessage, Typing, etc.), the `body` is
//! Signal Protocol encrypted ciphertext. For session-establishing types
//! (FriendRequest, FriendAccept), the fields are plaintext.

use serde::{Deserialize, Serialize};

use crate::error::{Result, TransportError};
use crate::frame::TypeId;

/// All DM-class payload variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DmPayload {
    /// Encrypted 1:1 chat message.
    DirectMessage {
        body: Vec<u8>,
        reply_to: Option<Vec<u8>>,
    },
    /// Typing indicator (ephemeral).
    Typing { typing: bool },
    /// Friend request (plaintext, TOFU signed).
    FriendRequest {
        display_name: String,
        message: String,
        prekey_bundle: Vec<u8>,
        profile_dht_key: String,
        route_blob: Vec<u8>,
        mailbox_dht_key: String,
        invite_id: Option<String>,
    },
    /// Friend request accepted.
    FriendAccept {
        prekey_bundle: Vec<u8>,
        profile_dht_key: String,
        route_blob: Vec<u8>,
        mailbox_dht_key: String,
        ephemeral_key: Vec<u8>,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
    },
    /// Friend request rejected.
    FriendReject,
    /// Delivery confirmation for a FriendRequest.
    FriendRequestAck,
    /// Notification that we have removed the peer as a friend.
    Unfriend,
    /// Acknowledgement of an Unfriend notification.
    UnfriendAck,
    /// Profile DHT key rotated (after block/unfriend).
    ProfileKeyRotated { new_profile_dht_key: String },
    /// Presence update (status, game info).
    PresenceUpdate {
        status: u8,
        game_info: Option<GamePresence>,
    },

    // ── W16.4 + W16.5b: 1:1 call signaling ──────────────────────────
    //
    // Hybrid `app_call` (invite/ringing) + `app_message` (user-decision).
    // Sender is implicit (envelope's `sender_key_hex`). The receiver
    // routes these to the call state machine (W16.5/W16.7).
    //
    // - CallInvite is RPC (`app_call`); receiver replies synchronously
    //   inside `app_call_reply` with `CallResponse::CallRinging`. See
    //   `payload/rpc.rs` for the request/response shape.
    // - CallAccept / CallDecline / CallEnd / CallMediaState /
    //   CallReaction travel as `app_message` because user-decision time
    //   is unbounded by Veilid's RPC budget.
    //
    // (DmPayload::CallInvite + DmPayload::CallRinging dropped per
    //  feedback_no_legacy_compat.md — pre-release; CallInvite is now
    //  in `payload::rpc::InboundCall::CallInvite`; CallRinging is now
    //  the synchronous reply payload `CallResponse::CallRinging`.)
    /// Receiver → caller: accepted. Carries the responder's X25519
    /// pub so the caller can derive the same call_key.
    CallAccept {
        call_id: String,
        acceptor_x25519_pub: Vec<u8>,
    },
    /// Receiver → caller: declined.
    CallDecline { call_id: String, reason: String },
    /// Hangup. Either party can send. Works for any state (Outgoing /
    /// Incoming / Connecting / Active).
    CallEnd { call_id: String, reason: String },
    /// Mid-call: peer toggled mic / camera / screen-share.
    CallMediaState {
        call_id: String,
        audio: bool,
        video: bool,
        screen: bool,
        timestamp_ms: u64,
    },
    /// Mid-call: emoji reaction.
    CallReaction {
        call_id: String,
        emoji: String,
        timestamp_ms: u64,
    },

    // ── W16.4: Group call signaling ─────────────────────────────────
    //
    // The initiator fans out one envelope PER invitee, each with that
    // invitee's per-recipient `wrapped_call_key`. See
    // `rekindle-calls::group` for the X25519 wrap logic.
    /// Caller → invitee: group call invite (per-invitee fan-out).
    GroupCallOffer {
        call_id: String,
        offer_kind: u8,
        initiator_x25519_pub: Vec<u8>,
        /// Hex Ed25519 pubkeys of every invitee. Receivers render the
        /// participant grid before they accept; late joins know who's
        /// expected.
        participants: Vec<String>,
        /// AES-256-GCM-wrapped 32-byte call_key for THIS recipient
        /// (40-byte wire format per `rekindle-calls::group`).
        wrapped_call_key: Vec<u8>,
        expires_at_ms: u64,
    },
    /// Invitee → caller: group call accept.
    GroupCallAccept { call_id: String },
    /// Invitee → caller: group call decline.
    GroupCallDecline { call_id: String, reason: String },

    // ── W16.4: DM invite (request/reply via expect-reply primitive) ─
    /// DM invite request — initiator awaits a reply with the new DM
    /// record key. Routed through `EnvelopeQueue::send_expect_reply`
    /// with a fresh `correlation_id`.
    DmInviteRequest {
        /// SMPL DM record key the initiator created.
        record_key: String,
        /// Receiver derives their slot keypair from this seed.
        slot_seed: Vec<u8>,
        /// Initiator's display name for this DM.
        alice_pseudonym: String,
        alice_subkey: u32,
        bob_subkey: u32,
    },
    /// DM invite reply — receiver's accept/decline. The reply
    /// envelope's `correlation_id` matches the request, and
    /// `EnvelopeQueue::deliver_reply` wakes the initiator's future.
    DmInviteReply {
        record_key: String,
        accepted: bool,
        /// Empty when accepted; populated with reason when declined.
        reason: String,
    },
    /// Group DM invite request.
    GroupDmInviteRequest {
        record_key: String,
        slot_seed: Vec<u8>,
        initiator_pseudonym: String,
        /// Hex pubkey + subkey for each member.
        participants: Vec<GroupDmParticipant>,
        /// MEK wrapped for THIS recipient (X25519 + AES-256-GCM).
        wrapped_mek: Vec<u8>,
        mek_generation: u32,
    },
    /// Group DM invite reply.
    GroupDmInviteReply {
        record_key: String,
        accepted: bool,
        reason: String,
    },
}

/// Member descriptor for a [`DmPayload::GroupDmInviteRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupDmParticipant {
    pub pseudonym: String,
    pub subkey: u32,
    /// Hex Ed25519 pubkey for verification.
    pub public_key: String,
}

/// Game presence information for DM presence updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamePresence {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    pub server_address: Option<String>,
}

// ── SubscriptionEvent conversion ───────────────────────────────────────

use rekindle_types::subscription_events::{
    ChannelMessageEvent, FriendEvent, PresenceEvent, SubscriptionEvent, TypingContext, TypingEvent,
};

impl DmPayload {
    /// Convert a DM payload into a `SubscriptionEvent` given sender context.
    ///
    /// Pure data transformation — no state mutation, no I/O, no logging.
    ///
    /// Returns `None` for variants that don't map to a generic
    /// subscription event (W16.4 call signaling, DM invites). Those
    /// payloads are consumed by the call state machine
    /// (`rekindle-transport::operations::calls`, W16.6/W16.7) and
    /// surface to subscribers via `TransportNotification` lifecycle
    /// variants (`CallStarted`, `IncomingCall`, etc.) instead.
    pub fn into_event(self, sender_key: &str, timestamp: u64) -> Option<SubscriptionEvent> {
        Some(match self {
            Self::DirectMessage { body, .. } => {
                SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
                    peer_key: sender_key.into(),
                    timestamp,
                    sender_name: None, // enriched from friend list by SubscriptionManager
                    body: Some(String::from_utf8_lossy(&body).to_string()),
                })
            }
            Self::Typing { typing } => {
                if typing {
                    SubscriptionEvent::Typing(TypingEvent::Started {
                        context: TypingContext::Dm {
                            peer_key: sender_key.into(),
                        },
                        who: sender_key.into(),
                    })
                } else {
                    SubscriptionEvent::Typing(TypingEvent::Stopped {
                        context: TypingContext::Dm {
                            peer_key: sender_key.into(),
                        },
                        who: sender_key.into(),
                    })
                }
            }
            Self::FriendRequest {
                display_name,
                message,
                ..
            } => SubscriptionEvent::Friend(FriendEvent::RequestReceived {
                from_key: sender_key.into(),
                display_name,
                message,
            }),
            Self::FriendAccept { .. } => SubscriptionEvent::Friend(FriendEvent::Accepted {
                peer_key: sender_key.into(),
                dm_log_key: String::new(),
            }),
            Self::FriendReject => SubscriptionEvent::Friend(FriendEvent::Rejected {
                peer_key: sender_key.into(),
            }),
            Self::FriendRequestAck => SubscriptionEvent::Friend(FriendEvent::RequestAcknowledged {
                peer_key: sender_key.into(),
            }),
            Self::Unfriend => SubscriptionEvent::Friend(FriendEvent::Removed {
                peer_key: sender_key.into(),
            }),
            Self::UnfriendAck => SubscriptionEvent::Friend(FriendEvent::RemoveAcknowledged {
                peer_key: sender_key.into(),
            }),
            Self::ProfileKeyRotated {
                new_profile_dht_key,
            } => SubscriptionEvent::Friend(FriendEvent::ProfileKeyRotated {
                peer_key: sender_key.into(),
                new_profile_dht_key,
            }),
            // W16.4 + W16.5b — call signaling and DM invites surface
            // via TransportNotification, not SubscriptionEvent. Caller
            // routes these elsewhere; into_event returns None.
            // (CallInvite + CallRinging dropped — see top of enum.)
            Self::CallAccept { .. }
            | Self::CallDecline { .. }
            | Self::CallEnd { .. }
            | Self::CallMediaState { .. }
            | Self::CallReaction { .. }
            | Self::GroupCallOffer { .. }
            | Self::GroupCallAccept { .. }
            | Self::GroupCallDecline { .. }
            | Self::DmInviteRequest { .. }
            | Self::DmInviteReply { .. }
            | Self::GroupDmInviteRequest { .. }
            | Self::GroupDmInviteReply { .. } => return None,
            Self::PresenceUpdate { status, game_info } => {
                let status_str = match status {
                    0 => "online",
                    1 => "away",
                    2 => "busy",
                    3 => "offline",
                    4 => "invisible",
                    _ => "unknown",
                };
                SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
                    peer_key: sender_key.into(),
                    status: status_str.into(),
                    game_name: game_info.map(|g| g.game_name),
                })
            }
        })
    }
}

/// Deserialize a DM payload from raw bytes based on the frame TypeId.
pub fn deserialize_dm(type_id: TypeId, bytes: &[u8]) -> Result<DmPayload> {
    postcard::from_bytes(bytes).map_err(|e| TransportError::DeserializationFailed {
        type_id: type_id as u8,
        reason: e.to_string(),
    })
}

/// Serialize a DM payload to bytes.
pub fn serialize_dm(payload: &DmPayload) -> Result<Vec<u8>> {
    postcard::to_stdvec(payload).map_err(|e| TransportError::SerializationFailed {
        reason: e.to_string(),
    })
}

/// Map a DmPayload variant to its frame TypeId.
pub fn dm_type_id(payload: &DmPayload) -> TypeId {
    match payload {
        DmPayload::DirectMessage { .. } => TypeId::DmMessage,
        DmPayload::Typing { .. } => TypeId::DmTyping,
        DmPayload::FriendRequest { .. } => TypeId::FriendRequest,
        DmPayload::FriendAccept { .. } => TypeId::FriendAccept,
        DmPayload::FriendReject => TypeId::FriendReject,
        DmPayload::FriendRequestAck => TypeId::FriendRequestAck,
        DmPayload::Unfriend => TypeId::Unfriend,
        DmPayload::UnfriendAck => TypeId::UnfriendAck,
        DmPayload::ProfileKeyRotated { .. } => TypeId::ProfileKeyRotated,
        DmPayload::PresenceUpdate { .. } => TypeId::DmPresenceUpdate,
        // W16.4 + W16.5b — 1:1 call signaling on app_message side
        // (CallInvite is RPC; see payload::rpc for that flow).
        DmPayload::CallAccept { .. } => TypeId::CallAccept,
        DmPayload::CallDecline { .. } => TypeId::CallDecline,
        DmPayload::CallEnd { .. } => TypeId::CallEnd,
        DmPayload::CallMediaState { .. } => TypeId::CallMediaState,
        DmPayload::CallReaction { .. } => TypeId::CallReaction,
        // W16.4 — group call signaling
        DmPayload::GroupCallOffer { .. } => TypeId::GroupCallOffer,
        DmPayload::GroupCallAccept { .. } => TypeId::GroupCallAccept,
        DmPayload::GroupCallDecline { .. } => TypeId::GroupCallDecline,
        // W16.4 — DM invite (expect-reply primitive)
        DmPayload::DmInviteRequest { .. } => TypeId::DmInviteRequest,
        DmPayload::DmInviteReply { .. } => TypeId::DmInviteReply,
        DmPayload::GroupDmInviteRequest { .. } => TypeId::GroupDmInviteRequest,
        DmPayload::GroupDmInviteReply { .. } => TypeId::GroupDmInviteReply,
    }
}
