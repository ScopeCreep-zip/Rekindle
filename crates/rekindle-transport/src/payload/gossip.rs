//! Gossip broadcast payload types.
//!
//! The outer [`SignedGossipEnvelope`] carries community routing metadata
//! (community_id, sender_pseudonym, TTL, Lamport timestamp) and an Ed25519
//! signature. The inner [`GossipPayload`] is the deserialized content.

use serde::{Deserialize, Serialize};

/// Signed gossip envelope — the wire format for community broadcasts.
///
/// Signature covers `payload_bytes` only. Routing fields (community_id,
/// sender_pseudonym, ttl, lamport_ts) are in the clear for dedup/routing
/// but the payload itself is authenticated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedGossipEnvelope {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub payload_bytes: Vec<u8>,
    pub signature: Vec<u8>,
    pub ttl: u8,
    pub lamport_ts: u64,
}

impl SignedGossipEnvelope {
    /// Compute a dedup key for this envelope.
    ///
    /// For message notifications: use the message_id.
    /// For typing/presence: use a time-bucketed key to collapse rapid updates.
    /// For everything else: BLAKE3 hash of the payload bytes.
    pub fn dedup_key(&self) -> String {
        // Try to extract a deterministic key from the payload
        if let Ok(payload) = postcard::from_bytes::<GossipPayload>(&self.payload_bytes) {
            match &payload {
                GossipPayload::MessageNotification { message_id, .. } => {
                    return message_id.clone();
                }
                GossipPayload::TypingIndicator { channel_id, .. } => {
                    let bucket = rekindle_utils::timestamp_secs() / 5;
                    return format!("typing:{channel_id}:{}:{bucket}", self.sender_pseudonym);
                }
                GossipPayload::PresenceUpdate { .. } => {
                    let bucket = rekindle_utils::timestamp_secs() / 30;
                    return format!("presence:{}:{bucket}", self.sender_pseudonym);
                }
                GossipPayload::Control(_) => {}
            }
        }
        // Fallback: BLAKE3 hash of payload bytes
        let hash = blake3::hash(&self.payload_bytes);
        hex::encode(&hash.as_bytes()[..16])
    }

    /// Whether this envelope carries a private payload that should NOT be forwarded.
    pub fn is_private(&self) -> bool {
        if let Ok(payload) = postcard::from_bytes::<GossipPayload>(&self.payload_bytes) {
            matches!(
                payload,
                GossipPayload::Control(
                    ControlPayload::JoinAccepted { .. }
                        | ControlPayload::JoinRejected { .. }
                        | ControlPayload::SlotKeypairGrant { .. }
                        | ControlPayload::AdminKeypairGrant { .. }
                        | ControlPayload::SyncResponse { .. }
                        | ControlPayload::KickedNotification
                )
            )
        } else {
            false
        }
    }
}

/// Inner gossip payload — the authenticated content of a broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipPayload {
    /// Notification that a new message exists in a channel SMPL record.
    /// Gossip carries the manifest (metadata), not the cargo (ciphertext).
    MessageNotification {
        channel_id: String,
        message_id: String,
        author_pseudonym: String,
        subkey_index: u32,
        lamport_ts: u64,
        sequence: u64,
        content_hash: String,
        timestamp: u64,
    },
    /// Member presence update.
    PresenceUpdate {
        pseudonym_key: String,
        status: String,
        game_name: Option<String>,
        game_id: Option<u32>,
        elapsed_seconds: Option<u32>,
        server_address: Option<String>,
        route_blob: Option<Vec<u8>>,
    },
    /// Typing indicator (ephemeral, not stored).
    TypingIndicator {
        channel_id: String,
        pseudonym_key: String,
    },
    /// A control operation.
    Control(ControlPayload),
}

/// All community control operations.
///
/// Every variant is fully typed — no `serde_json::Value` fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlPayload {
    // ── Member lifecycle ─────────────────────────────────────────
    MemberJoinRequest {
        pseudonym_key: String,
        display_name: String,
        invite_code: Option<String>,
        route_blob: Option<Vec<u8>>,
        prekey_bundle: Option<Vec<u8>>,
        claimed_subkey_index: Option<u32>,
    },
    MemberLeave {
        pseudonym_key: String,
    },
    JoinAccepted {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
        member_registry_key: Option<String>,
        slot_index: Option<u32>,
        wrapped_slot_seed: Option<Vec<u8>>,
    },
    JoinRejected {
        reason: String,
    },
    MemberJoined {
        pseudonym_key: String,
        display_name: String,
        role_ids: Vec<u32>,
        status: String,
        route_blob: Option<Vec<u8>>,
    },
    MemberRemoved {
        pseudonym_key: String,
    },

    // ── Moderation ───────────────────────────────────────────────
    Kick {
        target_pseudonym: String,
    },
    Ban {
        target_pseudonym: String,
    },
    Unban {
        target_pseudonym: String,
    },
    TimeoutMember {
        target_pseudonym: String,
        duration_seconds: u64,
        reason: Option<String>,
    },
    RemoveTimeout {
        target_pseudonym: String,
    },
    MemberTimedOut {
        pseudonym_key: String,
        timeout_until: Option<u64>,
    },

    // ── Messages ─────────────────────────────────────────────────
    MessageEdited {
        channel_id: String,
        message_id: String,
        new_ciphertext: Vec<u8>,
        mek_generation: u64,
        edited_at: u64,
    },
    MessageDeleted {
        channel_id: String,
        message_id: String,
    },

    // ── MEK management ───────────────────────────────────────────
    MekRotated {
        channel_id: Option<String>,
        new_generation: u64,
        rotator_pseudonym: Option<String>,
    },
    RequestMek {
        channel_id: String,
        needed_generation: u64,
        requester_pseudonym: String,
    },
    MekTransfer {
        community_id: String,
        channel_id: Option<String>,
        generation: u64,
        sender_pseudonym: String,
        wrapped_mek: Vec<u8>,
    },

    // ── Roles ────────────────────────────────────────────────────
    MemberRolesChanged {
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },
    OnboardingComplete {
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },

    // ── Channel permissions ──────────────────────────────────────
    ChannelOverwriteChanged {
        channel_id: String,
    },

    // ── Reactions & pins ─────────────────────────────────────────
    ReactionAdded {
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    ReactionRemoved {
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    MessagePinned {
        channel_id: String,
        message_id: String,
        pinned_by: String,
    },
    MessageUnpinned {
        channel_id: String,
        message_id: String,
    },

    // ── Events ───────────────────────────────────────────────────
    EventCreated {
        event: CommunityEvent,
    },
    EventUpdated {
        event: CommunityEvent,
    },
    EventDeleted {
        event_id: String,
    },
    EventRsvpChanged {
        event_id: String,
        pseudonym_key: String,
        status: String,
    },
    EventReminder {
        event_id: String,
        title: String,
        minutes_until_start: u32,
    },

    // ── Threads ──────────────────────────────────────────────────
    ThreadCreated {
        thread: ThreadInfo,
    },
    ThreadMessage {
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
        reply_to_id: Option<String>,
    },
    ThreadArchived {
        thread_id: String,
        archived: bool,
    },

    // ── Game servers ─────────────────────────────────────────────
    GameServerAdded {
        server: GameServerInfo,
    },
    GameServerRemoved {
        server_id: String,
    },

    // ── Governance ───────────────────────────────────────────────
    GovernanceUpdated {
        governance_key: String,
        subkey_index: u32,
        lamport_ts: u64,
    },

    // ── Voice signaling ──────────────────────────────────────────
    VoiceJoin {
        channel_id: String,
        route_blob: Vec<u8>,
    },
    VoiceLeave {
        channel_id: String,
    },
    VoiceModeSwitch {
        channel_id: String,
        mode: String,
        host_pseudonym: Option<String>,
    },
    VoiceMute {
        channel_id: String,
        target_pseudonym: String,
        muted: bool,
    },
    VoiceDeafen {
        channel_id: String,
        target_pseudonym: String,
        deafened: bool,
    },
    VoiceRoster {
        channel_id: String,
        participants: Vec<VoiceParticipant>,
    },

    // ── Admin delegation ─────────────────────────────────────────
    AdminKeypairGrant {
        wrapped_owner_keypair: Vec<u8>,
        wrapped_slot_seed: Vec<u8>,
    },
    SlotKeypairGrant {
        slot_index: u32,
        segment_index: u32,
        wrapped_slot_keypair: Vec<u8>,
    },

    // ── Bootstrap (via gossip — not the app_call bootstrap) ──────
    BootstrapRequest {
        joiner_pseudonym: String,
        governance_key: String,
    },
    BootstrapResponse {
        governance_entries: Vec<Vec<u8>>,
        member_list: Vec<Vec<u8>>,
        channel_meks: Vec<Vec<u8>>,
        recent_messages: Vec<Vec<u8>>,
        wrapped_owner_keypair: Vec<u8>,
    },

    // ── Sync ─────────────────────────────────────────────────────
    SyncRequest {
        channel_id: String,
        since_timestamp: u64,
    },
    SyncResponse {
        channel_id: String,
        messages: Vec<Vec<u8>>,
    },

    // ── System ───────────────────────────────────────────────────
    SystemMessage {
        body: String,
        timestamp: u64,
    },
    RaidAlert {
        active: bool,
    },
    ChannelLockdown {
        locked: bool,
    },
    KickedNotification,
    SubmitOnboardingAnswers {
        answers: Vec<OnboardingAnswer>,
    },
}

// ── Supporting types (fully typed, no serde_json::Value) ─────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityEvent {
    pub id: String,
    pub title: String,
    pub description: String,
    pub creator_pseudonym: String,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub channel_id: Option<String>,
    pub max_attendees: Option<u32>,
    pub created_at: u64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub starter_message_id: String,
    pub creator_pseudonym: String,
    pub created_at: u64,
    pub archived: bool,
    pub auto_archive_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameServerInfo {
    pub id: String,
    pub game_id: String,
    pub label: String,
    pub address: String,
    pub added_by: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceParticipant {
    pub pseudonym_key: String,
    pub route_blob: Vec<u8>,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingAnswer {
    pub question_id: String,
    pub selected_options: Vec<String>,
}

// ── SubscriptionEvent conversion ───────────────────────────────────────

use rekindle_types::subscription_events::{
    ChannelMessageEvent, CryptoEvent, GovernanceEvent, MembershipEvent, PresenceEvent, SocialEvent,
    SubscriptionEvent, SystemEvent, TypingContext, TypingEvent, VoiceEvent,
};

impl GossipPayload {
    /// Convert a gossip payload into a `SubscriptionEvent` given envelope context.
    ///
    /// This is a pure data transformation — no state mutation, no I/O, no logging.
    /// The compiler enforces exhaustiveness: adding a new `ControlPayload` variant
    /// without a match arm here is a build error.
    pub fn into_event(self, community: &str, sender: &str) -> SubscriptionEvent {
        let c = || community.to_string();
        let s = || sender.to_string();

        match self {
            Self::MessageNotification {
                channel_id,
                message_id,
                sequence,
                timestamp,
                ..
            } => {
                SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
                    community: c(),
                    channel: channel_id,
                    message_id,
                    sender_pseudonym: s(),
                    sequence,
                    timestamp,
                    body: None,              // populated by enrichment stage
                    reply_to_sequence: None, // populated by enrichment stage
                })
            }
            Self::TypingIndicator {
                channel_id,
                pseudonym_key,
            } => SubscriptionEvent::Typing(TypingEvent::Started {
                context: TypingContext::Channel {
                    community: c(),
                    channel: channel_id,
                },
                who: pseudonym_key,
            }),
            Self::PresenceUpdate {
                pseudonym_key,
                status,
                game_name,
                game_id,
                ..
            } => SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
                community: c(),
                pseudonym: pseudonym_key,
                status,
                game_name,
                game_id,
            }),
            Self::Control(ctrl) => ctrl.into_event(community, sender),
        }
    }
}

impl ControlPayload {
    /// Convert a control payload into a `SubscriptionEvent`.
    /// Exhaustive — every variant has a match arm.
    pub fn into_event(self, community: &str, sender: &str) -> SubscriptionEvent {
        let c = || community.to_string();
        let s = || sender.to_string();

        match self {
            // ── Member lifecycle ─────────────────────────────────
            Self::MemberJoinRequest {
                pseudonym_key,
                display_name,
                invite_code,
                ..
            } => SubscriptionEvent::Membership(MembershipEvent::JoinRequested {
                community: c(),
                pseudonym: pseudonym_key,
                display_name,
                has_invite: invite_code.is_some(),
            }),
            Self::MemberLeave { pseudonym_key } => {
                SubscriptionEvent::Membership(MembershipEvent::Left {
                    community: c(),
                    pseudonym: pseudonym_key,
                })
            }
            Self::JoinAccepted {
                mek_generation,
                slot_index,
                ..
            } => SubscriptionEvent::Membership(MembershipEvent::JoinAccepted {
                community: c(),
                mek_generation,
                slot_index,
            }),
            Self::JoinRejected { reason } => {
                SubscriptionEvent::Membership(MembershipEvent::JoinRejected {
                    community: c(),
                    reason,
                })
            }
            Self::MemberJoined {
                pseudonym_key,
                display_name,
                role_ids,
                ..
            } => SubscriptionEvent::Membership(MembershipEvent::Joined {
                community: c(),
                pseudonym: pseudonym_key,
                display_name,
                role_ids,
            }),
            Self::MemberRemoved { pseudonym_key } => {
                SubscriptionEvent::Membership(MembershipEvent::Removed {
                    community: c(),
                    pseudonym: pseudonym_key,
                })
            }

            // ── Moderation ──────────────────────────────────────
            Self::Kick { target_pseudonym } => {
                SubscriptionEvent::Membership(MembershipEvent::Kicked {
                    community: c(),
                    target_pseudonym,
                })
            }
            Self::Ban { target_pseudonym } => {
                SubscriptionEvent::Membership(MembershipEvent::Banned {
                    community: c(),
                    target_pseudonym,
                })
            }
            Self::Unban { target_pseudonym } => {
                SubscriptionEvent::Membership(MembershipEvent::Unbanned {
                    community: c(),
                    target_pseudonym,
                })
            }
            Self::TimeoutMember {
                target_pseudonym,
                duration_seconds,
                reason,
            } => SubscriptionEvent::Membership(MembershipEvent::TimedOut {
                community: c(),
                target_pseudonym,
                duration_seconds,
                reason,
            }),
            Self::RemoveTimeout { target_pseudonym } => {
                SubscriptionEvent::Membership(MembershipEvent::TimeoutRemoved {
                    community: c(),
                    target_pseudonym,
                })
            }
            Self::MemberTimedOut {
                pseudonym_key,
                timeout_until,
            } => SubscriptionEvent::Membership(MembershipEvent::TimeoutStatusChanged {
                community: c(),
                pseudonym: pseudonym_key,
                timeout_until,
            }),

            // ── Messages ────────────────────────────────────────
            Self::MessageEdited {
                channel_id,
                message_id,
                edited_at,
                ..
            } => SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Edited {
                community: c(),
                channel: channel_id,
                message_id,
                edited_at,
                body: None, // populated by enrichment stage
            }),
            Self::MessageDeleted {
                channel_id,
                message_id,
            } => SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Deleted {
                community: c(),
                channel: channel_id,
                message_id,
            }),

            // ── MEK management ──────────────────────────────────
            Self::MekRotated {
                channel_id,
                new_generation,
                rotator_pseudonym,
            } => SubscriptionEvent::Crypto(CryptoEvent::MekRotated {
                community: c(),
                channel: channel_id,
                generation: new_generation,
                rotator_pseudonym,
            }),
            Self::RequestMek {
                channel_id,
                needed_generation,
                requester_pseudonym,
            } => SubscriptionEvent::Crypto(CryptoEvent::MekRequested {
                community: c(),
                channel: channel_id,
                needed_generation,
                requester_pseudonym,
            }),
            Self::MekTransfer {
                community_id,
                channel_id,
                generation,
                sender_pseudonym,
                ..
            } => SubscriptionEvent::Crypto(CryptoEvent::MekTransferred {
                community: community_id,
                channel: channel_id,
                generation,
                sender_pseudonym,
            }),

            // ── Roles ───────────────────────────────────────────
            Self::MemberRolesChanged {
                pseudonym_key,
                role_ids,
            } => SubscriptionEvent::Membership(MembershipEvent::RolesChanged {
                community: c(),
                pseudonym: pseudonym_key,
                role_ids,
            }),
            Self::OnboardingComplete {
                pseudonym_key,
                role_ids,
            } => SubscriptionEvent::Membership(MembershipEvent::OnboardingCompleted {
                community: c(),
                pseudonym: pseudonym_key,
                role_ids,
            }),
            Self::SubmitOnboardingAnswers { answers } => {
                SubscriptionEvent::Membership(MembershipEvent::OnboardingAnswersSubmitted {
                    community: c(),
                    sender_pseudonym: s(),
                    answer_count: answers.len(),
                })
            }

            // ── Channel permissions ─────────────────────────────
            Self::ChannelOverwriteChanged { channel_id } => {
                SubscriptionEvent::Governance(GovernanceEvent::ChannelPermissionsChanged {
                    community: c(),
                    channel: channel_id,
                })
            }

            // ── Reactions & pins ────────────────────────────────
            Self::ReactionAdded {
                channel_id,
                message_id,
                emoji,
                reactor_pseudonym,
            } => SubscriptionEvent::Social(SocialEvent::ReactionAdded {
                community: c(),
                channel: channel_id,
                message_id,
                emoji,
                reactor_pseudonym,
            }),
            Self::ReactionRemoved {
                channel_id,
                message_id,
                emoji,
                reactor_pseudonym,
            } => SubscriptionEvent::Social(SocialEvent::ReactionRemoved {
                community: c(),
                channel: channel_id,
                message_id,
                emoji,
                reactor_pseudonym,
            }),
            Self::MessagePinned {
                channel_id,
                message_id,
                pinned_by,
            } => SubscriptionEvent::Social(SocialEvent::MessagePinned {
                community: c(),
                channel: channel_id,
                message_id,
                pinned_by,
            }),
            Self::MessageUnpinned {
                channel_id,
                message_id,
            } => SubscriptionEvent::Social(SocialEvent::MessageUnpinned {
                community: c(),
                channel: channel_id,
                message_id,
            }),

            // ── Events ──────────────────────────────────────────
            Self::EventCreated { event } => SubscriptionEvent::Social(SocialEvent::EventCreated {
                community: c(),
                event_id: event.id,
                title: event.title,
                start_time: event.start_time,
            }),
            Self::EventUpdated { event } => SubscriptionEvent::Social(SocialEvent::EventUpdated {
                community: c(),
                event_id: event.id,
                title: event.title,
            }),
            Self::EventDeleted { event_id } => {
                SubscriptionEvent::Social(SocialEvent::EventDeleted {
                    community: c(),
                    event_id,
                })
            }
            Self::EventRsvpChanged {
                event_id,
                pseudonym_key,
                status,
            } => SubscriptionEvent::Social(SocialEvent::EventRsvpChanged {
                community: c(),
                event_id,
                pseudonym: pseudonym_key,
                rsvp_status: status,
            }),
            Self::EventReminder {
                event_id,
                title,
                minutes_until_start,
            } => SubscriptionEvent::Social(SocialEvent::EventReminder {
                community: c(),
                event_id,
                title,
                minutes_until_start,
            }),

            // ── Threads ─────────────────────────────────────────
            Self::ThreadCreated { thread } => {
                SubscriptionEvent::Social(SocialEvent::ThreadCreated {
                    community: c(),
                    channel: thread.channel_id,
                    thread_id: thread.id,
                    thread_name: thread.name,
                    creator_pseudonym: thread.creator_pseudonym,
                })
            }
            Self::ThreadMessage {
                thread_id,
                message_id,
                sender_pseudonym,
                timestamp,
                ..
            } => SubscriptionEvent::Social(SocialEvent::ThreadMessagePosted {
                community: c(),
                thread_id,
                message_id,
                sender_pseudonym,
                timestamp,
            }),
            Self::ThreadArchived {
                thread_id,
                archived,
            } => SubscriptionEvent::Social(SocialEvent::ThreadArchiveChanged {
                community: c(),
                thread_id,
                archived,
            }),

            // ── Game servers ────────────────────────────────────
            Self::GameServerAdded { server } => {
                SubscriptionEvent::Social(SocialEvent::GameServerAdded {
                    community: c(),
                    server_id: server.id,
                    game_id: server.game_id,
                    label: server.label,
                })
            }
            Self::GameServerRemoved { server_id } => {
                SubscriptionEvent::Social(SocialEvent::GameServerRemoved {
                    community: c(),
                    server_id,
                })
            }

            // ── Governance ──────────────────────────────────────
            Self::GovernanceUpdated {
                subkey_index,
                lamport_ts,
                ..
            } => SubscriptionEvent::Governance(GovernanceEvent::GovernanceSubkeyUpdated {
                community: c(),
                subkey_index,
                lamport_ts,
            }),

            // ── Voice signaling ─────────────────────────────────
            Self::VoiceJoin { channel_id, .. } => SubscriptionEvent::Voice(VoiceEvent::Joined {
                community: c(),
                channel: channel_id,
                pseudonym: s(),
            }),
            Self::VoiceLeave { channel_id } => SubscriptionEvent::Voice(VoiceEvent::Left {
                community: c(),
                channel: channel_id,
                pseudonym: s(),
            }),
            Self::VoiceModeSwitch {
                channel_id,
                mode,
                host_pseudonym,
            } => SubscriptionEvent::Voice(VoiceEvent::ModeChanged {
                community: c(),
                channel: channel_id,
                mode,
                host_pseudonym,
            }),
            Self::VoiceMute {
                channel_id,
                target_pseudonym,
                muted,
            } => SubscriptionEvent::Voice(VoiceEvent::MuteChanged {
                community: c(),
                channel: channel_id,
                target_pseudonym,
                muted,
            }),
            Self::VoiceDeafen {
                channel_id,
                target_pseudonym,
                deafened,
            } => SubscriptionEvent::Voice(VoiceEvent::DeafenChanged {
                community: c(),
                channel: channel_id,
                target_pseudonym,
                deafened,
            }),
            Self::VoiceRoster {
                channel_id,
                participants,
            } => SubscriptionEvent::Voice(VoiceEvent::RosterUpdated {
                community: c(),
                channel: channel_id,
                participant_count: participants.len(),
            }),

            // ── Admin delegation ────────────────────────────────
            Self::AdminKeypairGrant { .. } => {
                SubscriptionEvent::Crypto(CryptoEvent::AdminKeypairGranted { community: c() })
            }
            Self::SlotKeypairGrant {
                slot_index,
                segment_index,
                ..
            } => SubscriptionEvent::Crypto(CryptoEvent::SlotKeypairGranted {
                community: c(),
                slot_index,
                segment_index,
            }),

            // ── Bootstrap ───────────────────────────────────────
            Self::BootstrapRequest {
                joiner_pseudonym, ..
            } => SubscriptionEvent::System(SystemEvent::BootstrapRequested {
                community: c(),
                joiner_pseudonym,
            }),
            Self::BootstrapResponse { .. } => {
                SubscriptionEvent::System(SystemEvent::BootstrapReceived { community: c() })
            }

            // ── Sync ────────────────────────────────────────────
            Self::SyncRequest {
                channel_id,
                since_timestamp,
            } => SubscriptionEvent::System(SystemEvent::SyncRequested {
                community: c(),
                channel: channel_id,
                since_timestamp,
            }),
            Self::SyncResponse {
                channel_id,
                messages,
            } => SubscriptionEvent::System(SystemEvent::SyncReceived {
                community: c(),
                channel: channel_id,
                message_count: messages.len(),
            }),

            // ── System ──────────────────────────────────────────
            Self::SystemMessage { body, timestamp } => {
                SubscriptionEvent::System(SystemEvent::Announcement {
                    community: Some(c()),
                    body,
                    timestamp,
                })
            }
            Self::RaidAlert { active } => SubscriptionEvent::System(SystemEvent::RaidAlert {
                community: c(),
                active,
            }),
            Self::ChannelLockdown { locked } => {
                SubscriptionEvent::System(SystemEvent::ChannelLockdown {
                    community: c(),
                    locked,
                })
            }
            Self::KickedNotification => {
                SubscriptionEvent::System(SystemEvent::Kicked { community: c() })
            }
        }
    }
}
