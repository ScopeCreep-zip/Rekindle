//! Community gossip broadcast — every GossipPayload and ControlPayload variant.
//!
//! Each public function:
//! 1. Builds the typed `GossipPayload`
//! 2. Serializes via postcard
//! 3. Increments the community Lamport clock
//! 4. Signs with the community pseudonym Ed25519 key
//! 5. Fans out to mesh peers via `Sender::broadcast_gossip`
//!
//! Rate limits are enforced for ephemeral signals (typing, presence).
//! Persistent signals (messages, membership) are never rate-limited.

use std::collections::HashMap;

use parking_lot::RwLock;
use tracing::{debug, trace, warn};

use super::node::TransportNode;
use super::send::BroadcastReport;
use crate::crypto::envelope;
use crate::gossip::GossipMesh;
use crate::payload::gossip::{
    CommunityEvent, ControlPayload, GameServerInfo, GossipPayload, OnboardingAnswer, ThreadInfo,
    VoiceParticipant,
};

use super::OutboundRateLimiter;

/// Type alias for the community gossip mesh map to satisfy clippy::implicit_hasher.
pub type MeshMap = HashMap<String, GossipMesh>;

/// Default TTL for gossip broadcasts.
const DEFAULT_TTL: u8 = 3;
/// Minimum interval between typing broadcasts per (community, channel).
const TYPING_RATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);
/// Minimum interval between presence broadcasts per sender.
const PRESENCE_RATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

// ── Top-level GossipPayload variants ───────────────────────────────────

/// Broadcast a `MessageNotification` after appending to a channel DhtLog.
pub async fn message_notification(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    author_pseudonym: &str,
    sequence: u64,
    content_hash: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::MessageNotification {
        channel_id: channel_id.into(),
        message_id: message_id.into(),
        author_pseudonym: author_pseudonym.into(),
        subkey_index: 0,
        lamport_ts: 0,
        sequence,
        content_hash: content_hash.into(),
        timestamp: rekindle_utils::timestamp_ms(),
    };
    build_sign_send(
        node,
        meshes,
        community_id,
        author_pseudonym,
        signing_key,
        payload,
    )
    .await
}

/// Broadcast a `TypingIndicator`. Rate-limited: max 1 per 3s per (community, channel).
pub async fn typing_indicator(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    rate_limiter: &RwLock<OutboundRateLimiter>,
    community_id: &str,
    channel_id: &str,
    pseudonym_key: &str,
    signing_key: &[u8; 32],
) -> Option<BroadcastReport> {
    let key = format!("{community_id}:typing:{channel_id}");
    if !rate_limiter.write().check(&key, TYPING_RATE_LIMIT) {
        trace!(community_id, channel_id, "typing broadcast rate-limited");
        return None;
    }
    let payload = GossipPayload::TypingIndicator {
        channel_id: channel_id.into(),
        pseudonym_key: pseudonym_key.into(),
    };
    Some(
        build_sign_send(
            node,
            meshes,
            community_id,
            pseudonym_key,
            signing_key,
            payload,
        )
        .await,
    )
}

/// Broadcast a `PresenceUpdate`. Rate-limited: max 1 per 30s per sender.
pub async fn presence_update(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    rate_limiter: &RwLock<OutboundRateLimiter>,
    community_id: &str,
    pseudonym_key: &str,
    status: &str,
    game_name: Option<&str>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<&str>,
    route_blob: Option<Vec<u8>>,
    signing_key: &[u8; 32],
) -> Option<BroadcastReport> {
    let key = format!("{community_id}:presence:{pseudonym_key}");
    if !rate_limiter.write().check(&key, PRESENCE_RATE_LIMIT) {
        trace!(community_id, "presence broadcast rate-limited");
        return None;
    }
    let payload = GossipPayload::PresenceUpdate {
        pseudonym_key: pseudonym_key.into(),
        status: status.into(),
        game_name: game_name.map(String::from),
        game_id,
        elapsed_seconds,
        server_address: server_address.map(String::from),
        route_blob,
    };
    Some(
        build_sign_send(
            node,
            meshes,
            community_id,
            pseudonym_key,
            signing_key,
            payload,
        )
        .await,
    )
}

// ── Member lifecycle controls ──────────────────────────────────────────

pub async fn member_join_request(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    invite_code: Option<&str>,
    route_blob: Option<Vec<u8>>,
    prekey_bundle: Option<Vec<u8>>,
    claimed_subkey_index: Option<u32>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::Control(ControlPayload::MemberJoinRequest {
        pseudonym_key: pseudonym_key.into(),
        display_name: display_name.into(),
        invite_code: invite_code.map(String::from),
        route_blob,
        prekey_bundle,
        claimed_subkey_index,
    });
    build_sign_send(
        node,
        meshes,
        community_id,
        pseudonym_key,
        signing_key,
        payload,
    )
    .await
}

pub async fn member_leave(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    pseudonym_key: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::Control(ControlPayload::MemberLeave {
        pseudonym_key: pseudonym_key.into(),
    });
    build_sign_send(
        node,
        meshes,
        community_id,
        pseudonym_key,
        signing_key,
        payload,
    )
    .await
}

pub async fn member_joined(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender_pseudonym: &str,
    pseudonym_key: &str,
    display_name: &str,
    role_ids: Vec<u32>,
    status: &str,
    route_blob: Option<Vec<u8>>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::Control(ControlPayload::MemberJoined {
        pseudonym_key: pseudonym_key.into(),
        display_name: display_name.into(),
        role_ids,
        status: status.into(),
        route_blob,
    });
    build_sign_send(
        node,
        meshes,
        community_id,
        sender_pseudonym,
        signing_key,
        payload,
    )
    .await
}

pub async fn member_removed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender_pseudonym: &str,
    target_pseudonym: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    let payload = GossipPayload::Control(ControlPayload::MemberRemoved {
        pseudonym_key: target_pseudonym.into(),
    });
    build_sign_send(
        node,
        meshes,
        community_id,
        sender_pseudonym,
        signing_key,
        payload,
    )
    .await
}

// ── Moderation controls ────────────────────────────────────────────────

pub async fn kick(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::Kick {
            target_pseudonym: target.into(),
        },
    )
    .await
}

pub async fn ban(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::Ban {
            target_pseudonym: target.into(),
        },
    )
    .await
}

pub async fn unban(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::Unban {
            target_pseudonym: target.into(),
        },
    )
    .await
}

pub async fn timeout_member(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    duration_seconds: u64,
    reason: Option<&str>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::TimeoutMember {
            target_pseudonym: target.into(),
            duration_seconds,
            reason: reason.map(String::from),
        },
    )
    .await
}

pub async fn remove_timeout(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    target: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::RemoveTimeout {
            target_pseudonym: target.into(),
        },
    )
    .await
}

pub async fn member_timed_out(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    pseudonym: &str,
    timeout_until: Option<u64>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MemberTimedOut {
            pseudonym_key: pseudonym.into(),
            timeout_until,
        },
    )
    .await
}

// ── Message controls ───────────────────────────────────────────────────

pub async fn message_edited(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    new_ciphertext: Vec<u8>,
    mek_generation: u64,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MessageEdited {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            new_ciphertext,
            mek_generation,
            edited_at: rekindle_utils::timestamp_ms(),
        },
    )
    .await
}

pub async fn message_deleted(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MessageDeleted {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
        },
    )
    .await
}

// ── MEK management ─────────────────────────────────────────────────────

pub async fn mek_rotated(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: Option<&str>,
    new_generation: u64,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MekRotated {
            channel_id: channel_id.map(String::from),
            new_generation,
            rotator_pseudonym: Some(sender.into()),
        },
    )
    .await
}

pub async fn request_mek(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    needed_generation: u64,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::RequestMek {
            channel_id: channel_id.into(),
            needed_generation,
            requester_pseudonym: sender.into(),
        },
    )
    .await
}

pub async fn mek_transfer(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: Option<&str>,
    generation: u64,
    wrapped_mek: Vec<u8>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MekTransfer {
            community_id: community_id.into(),
            channel_id: channel_id.map(String::from),
            generation,
            sender_pseudonym: sender.into(),
            wrapped_mek,
        },
    )
    .await
}

// ── Roles ──────────────────────────────────────────────────────────────

pub async fn member_roles_changed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    pseudonym: &str,
    role_ids: Vec<u32>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MemberRolesChanged {
            pseudonym_key: pseudonym.into(),
            role_ids,
        },
    )
    .await
}

pub async fn onboarding_complete(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    pseudonym: &str,
    role_ids: Vec<u32>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::OnboardingComplete {
            pseudonym_key: pseudonym.into(),
            role_ids,
        },
    )
    .await
}

pub async fn submit_onboarding_answers(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    answers: Vec<OnboardingAnswer>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SubmitOnboardingAnswers { answers },
    )
    .await
}

// ── Channel permissions ────────────────────────────────────────────────

pub async fn channel_overwrite_changed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ChannelOverwriteChanged {
            channel_id: channel_id.into(),
        },
    )
    .await
}

// ── Reactions & pins ───────────────────────────────────────────────────

pub async fn reaction_added(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    emoji: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ReactionAdded {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            emoji: emoji.into(),
            reactor_pseudonym: sender.into(),
        },
    )
    .await
}

pub async fn reaction_removed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    emoji: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ReactionRemoved {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            emoji: emoji.into(),
            reactor_pseudonym: sender.into(),
        },
    )
    .await
}

pub async fn message_pinned(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MessagePinned {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            pinned_by: sender.into(),
        },
    )
    .await
}

pub async fn message_unpinned(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    message_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::MessageUnpinned {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
        },
    )
    .await
}

// ── Events (community scheduled events) ────────────────────────────────

pub async fn event_created(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    event: CommunityEvent,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::EventCreated { event },
    )
    .await
}

pub async fn event_updated(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    event: CommunityEvent,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::EventUpdated { event },
    )
    .await
}

pub async fn event_deleted(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    event_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::EventDeleted {
            event_id: event_id.into(),
        },
    )
    .await
}

pub async fn event_rsvp_changed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    event_id: &str,
    rsvp_status: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::EventRsvpChanged {
            event_id: event_id.into(),
            pseudonym_key: sender.into(),
            status: rsvp_status.into(),
        },
    )
    .await
}

pub async fn event_reminder(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    event_id: &str,
    title: &str,
    minutes_until_start: u32,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::EventReminder {
            event_id: event_id.into(),
            title: title.into(),
            minutes_until_start,
        },
    )
    .await
}

// ── Threads ────────────────────────────────────────────────────────────

pub async fn thread_created(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    thread: ThreadInfo,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ThreadCreated { thread },
    )
    .await
}

pub async fn thread_message(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    thread_id: &str,
    message_id: &str,
    ciphertext: Vec<u8>,
    mek_generation: u64,
    reply_to_id: Option<&str>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ThreadMessage {
            thread_id: thread_id.into(),
            message_id: message_id.into(),
            sender_pseudonym: sender.into(),
            ciphertext,
            mek_generation,
            timestamp: rekindle_utils::timestamp_ms(),
            reply_to_id: reply_to_id.map(String::from),
        },
    )
    .await
}

pub async fn thread_archived(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    thread_id: &str,
    archived: bool,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ThreadArchived {
            thread_id: thread_id.into(),
            archived,
        },
    )
    .await
}

// ── Game servers ───────────────────────────────────────────────────────

pub async fn game_server_added(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    server: GameServerInfo,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::GameServerAdded { server },
    )
    .await
}

pub async fn game_server_removed(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    server_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::GameServerRemoved {
            server_id: server_id.into(),
        },
    )
    .await
}

// ── Governance ─────────────────────────────────────────────────────────

pub async fn governance_updated(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    governance_key: &str,
    subkey_index: u32,
    lamport_ts: u64,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::GovernanceUpdated {
            governance_key: governance_key.into(),
            subkey_index,
            lamport_ts,
        },
    )
    .await
}

// ── Voice signaling ────────────────────────────────────────────────────

pub async fn voice_join(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    route_blob: Vec<u8>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::VoiceJoin {
            channel_id: channel_id.into(),
            route_blob,
        },
    )
    .await
}

pub async fn voice_leave(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::VoiceLeave {
            channel_id: channel_id.into(),
        },
    )
    .await
}

pub async fn voice_mode_switch(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    mode: &str,
    host_pseudonym: Option<&str>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::VoiceModeSwitch {
            channel_id: channel_id.into(),
            mode: mode.into(),
            host_pseudonym: host_pseudonym.map(String::from),
        },
    )
    .await
}

pub async fn voice_mute(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    target: &str,
    muted: bool,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::VoiceMute {
            channel_id: channel_id.into(),
            target_pseudonym: target.into(),
            muted,
        },
    )
    .await
}

pub async fn voice_deafen(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    target: &str,
    deafened: bool,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::VoiceDeafen {
            channel_id: channel_id.into(),
            target_pseudonym: target.into(),
            deafened,
        },
    )
    .await
}

pub async fn voice_roster(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    participants: Vec<VoiceParticipant>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::VoiceRoster {
            channel_id: channel_id.into(),
            participants,
        },
    )
    .await
}

// ── Admin delegation (private — not forwarded by mesh) ─────────────────

pub async fn admin_keypair_grant(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    wrapped_owner_keypair: Vec<u8>,
    wrapped_slot_seed: Vec<u8>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::AdminKeypairGrant {
            wrapped_owner_keypair,
            wrapped_slot_seed,
        },
    )
    .await
}

pub async fn slot_keypair_grant(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    slot_index: u32,
    segment_index: u32,
    wrapped_slot_keypair: Vec<u8>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SlotKeypairGrant {
            slot_index,
            segment_index,
            wrapped_slot_keypair,
        },
    )
    .await
}

// ── Private payloads (JoinAccepted, JoinRejected — targeted, not mesh) ─

pub async fn join_accepted(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    mek_encrypted: Vec<u8>,
    mek_generation: u64,
    registry_key: Option<&str>,
    slot_index: Option<u32>,
    wrapped_slot_seed: Option<Vec<u8>>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::JoinAccepted {
            mek_encrypted,
            mek_generation,
            member_registry_key: registry_key.map(String::from),
            slot_index,
            wrapped_slot_seed,
        },
    )
    .await
}

pub async fn join_rejected(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    reason: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::JoinRejected {
            reason: reason.into(),
        },
    )
    .await
}

// ── Bootstrap & sync ───────────────────────────────────────────────────

pub async fn bootstrap_request(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    joiner_pseudonym: &str,
    governance_key: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::BootstrapRequest {
            joiner_pseudonym: joiner_pseudonym.into(),
            governance_key: governance_key.into(),
        },
    )
    .await
}

pub async fn bootstrap_response(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    governance_entries: Vec<Vec<u8>>,
    member_list: Vec<Vec<u8>>,
    channel_meks: Vec<Vec<u8>>,
    recent_messages: Vec<Vec<u8>>,
    wrapped_owner_keypair: Vec<u8>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::BootstrapResponse {
            governance_entries,
            member_list,
            channel_meks,
            recent_messages,
            wrapped_owner_keypair,
        },
    )
    .await
}

pub async fn sync_request(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    since_timestamp: u64,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SyncRequest {
            channel_id: channel_id.into(),
            since_timestamp,
        },
    )
    .await
}

pub async fn sync_response(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    messages: Vec<Vec<u8>>,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SyncResponse {
            channel_id: channel_id.into(),
            messages,
        },
    )
    .await
}

// ── System ─────────────────────────────────────────────────────────────

pub async fn system_message(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    body: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::SystemMessage {
            body: body.into(),
            timestamp: rekindle_utils::timestamp_ms(),
        },
    )
    .await
}

pub async fn raid_alert(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    active: bool,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::RaidAlert { active },
    )
    .await
}

pub async fn channel_lockdown(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    locked: bool,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::ChannelLockdown { locked },
    )
    .await
}

pub async fn kicked_notification(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    signing_key: &[u8; 32],
) -> BroadcastReport {
    control(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        ControlPayload::KickedNotification,
    )
    .await
}

// ── Internal helpers ───────────────────────────────────────────────────

/// Shortcut for broadcasting a ControlPayload wrapped in GossipPayload::Control.
async fn control(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    signing_key: &[u8; 32],
    ctrl: ControlPayload,
) -> BroadcastReport {
    build_sign_send(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        GossipPayload::Control(ctrl),
    )
    .await
}

/// Build a signed gossip envelope and fan out to mesh peers.
async fn build_sign_send(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender_pseudonym: &str,
    signing_key: &[u8; 32],
    payload: GossipPayload,
) -> BroadcastReport {
    // Serialize inner payload
    let payload_bytes = match postcard::to_stdvec(&payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "gossip broadcast: payload serialization failed");
            return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("serialize: {e}"))],
            };
        }
    };

    // Increment Lamport clock and collect peer route blobs
    let (lamport_ts, peer_targets) = {
        let mut guard = meshes.write();
        let Some(mesh) = guard.get_mut(community_id) else {
            debug!(community_id, "gossip broadcast: no mesh for community");
            return BroadcastReport::default();
        };
        let ts = mesh.clock.increment();
        let targets: Vec<(String, Vec<u8>)> = mesh
            .peers
            .iter()
            .map(|(k, m)| (k.clone(), m.route_blob.clone()))
            .collect();
        (ts, targets)
    };

    if peer_targets.is_empty() {
        trace!(community_id, "gossip broadcast: no peers in mesh");
        return BroadcastReport::default();
    }

    // Sign the envelope
    let signing = ed25519_dalek::SigningKey::from_bytes(signing_key);
    let envelope = envelope::sign_gossip_envelope(
        &signing,
        community_id,
        sender_pseudonym,
        &payload_bytes,
        DEFAULT_TTL,
        lamport_ts,
    );

    // Build framed targets and broadcast
    let sender = node.sender();
    let mut targets_with_routes = Vec::with_capacity(peer_targets.len());
    for (key, blob) in &peer_targets {
        match node.import_route(blob) {
            Ok(target) => targets_with_routes.push((key.clone(), target)),
            Err(e) => debug!(peer = %key, error = %e, "gossip broadcast: route import failed"),
        }
    }

    sender
        .broadcast_gossip(&targets_with_routes, &envelope)
        .await
}

/// Send a signed gossip envelope directly to a single target via their route.
///
/// Used for point-to-point notifications (e.g., JoinAccepted to a specific joiner)
/// where the target is NOT in the gossip mesh. Bypasses mesh lookup entirely.
///
/// This is the tier 2 (direct notification) primitive for authoritative state changes.
pub async fn send_direct(
    node: &TransportNode,
    community_id: &str,
    sender_pseudonym: &str,
    signing_key: &[u8; 32],
    payload: GossipPayload,
    target_key: &str,
    target_route_blob: &[u8],
) -> BroadcastReport {
    let payload_bytes = match postcard::to_stdvec(&payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "direct gossip: payload serialization failed");
            return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("serialize: {e}"))],
            };
        }
    };

    let signing = ed25519_dalek::SigningKey::from_bytes(signing_key);
    let envelope = envelope::sign_gossip_envelope(
        &signing,
        community_id,
        sender_pseudonym,
        &payload_bytes,
        0,
        0, // TTL=0 (no forwarding), lamport=0 (single-shot)
    );

    let target = match node.import_route(target_route_blob) {
        Ok(t) => t,
        Err(e) => {
            debug!(target = target_key, error = %e, "direct gossip: route import failed");
            return BroadcastReport {
                delivered: 0,
                failures: vec![(target_key.into(), format!("{e}"))],
            };
        }
    };

    node.sender()
        .broadcast_gossip(&[(target_key.to_string(), target)], &envelope)
        .await
}
