//! Data-transfer types carried inside [`super::CommunityEvent`] variants:
//! channel/category snapshots, role DTOs, and the `rekindle-types` aliases
//! for events/threads/game-servers, plus the `From` conversions that build
//! a `RoleDto` from the various in-crate and protocol role shapes.

use serde::{Deserialize, Serialize};

/// Event info DTO for frontend consumption (architecture §21).
///
/// Type alias for the canonical wire/in-memory shape defined in
/// `rekindle-types::event::EventInfo`. The DTO name is preserved so
/// existing call sites and IPC consumers don't need to be renamed
/// during the typed-envelope migration (plan:
/// `.claude/plans/community-envelope-capnp-migration.md`).
pub type EventInfoDto = rekindle_types::event::EventInfo;

/// RSVP entry DTO — alias for `rekindle-types::event::EventRsvp`.
pub type EventRsvpInfoDto = rekindle_types::event::EventRsvp;

/// Role DTO for frontend consumption (mirrors protocol's `RoleDto`).
///
/// `permissions` is serialized as a string to avoid JavaScript `Number` precision
/// loss — `u64` values above `2^53 - 1` lose low bits when parsed as JSON numbers,
/// which silently strips the ADMINISTRATOR flag (bit 3) from the Owner role.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDto {
    pub id: u32,
    pub name: String,
    pub color: u32,
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_u64_as_string",
        deserialize_with = "crate::serde_helpers::deserialize_u64_from_string_or_number"
    )]
    pub permissions: u64,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    #[serde(default)]
    pub self_assignable: bool,
    /// Architecture §19.4 — when set, only one role per group may be active per member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_group: Option<String>,
}

impl From<&rekindle_protocol::messaging::RoleDto> for RoleDto {
    fn from(dto: &rekindle_protocol::messaging::RoleDto) -> Self {
        Self {
            id: dto.id,
            name: dto.name.clone(),
            color: dto.color,
            permissions: dto.permissions,
            position: dto.position,
            hoist: dto.hoist,
            mentionable: dto.mentionable,
            self_assignable: dto.self_assignable,
            exclusion_group: None,
        }
    }
}

/// Thread info DTO for frontend consumption.
/// Thread info DTO — alias for `rekindle-types::thread::ThreadInfo`.
pub type ThreadInfoDto = rekindle_types::thread::ThreadInfo;

/// Game server info DTO — alias for
/// `rekindle-types::game_server::GameServerInfo`.
pub type GameServerInfoDto = rekindle_types::game_server::GameServerInfo;

impl From<&crate::state::RoleDefinition> for RoleDto {
    fn from(def: &crate::state::RoleDefinition) -> Self {
        Self {
            id: def.id,
            name: def.name.clone(),
            color: def.color,
            permissions: def.permissions,
            position: def.position,
            hoist: def.hoist,
            mentionable: def.mentionable,
            self_assignable: def.self_assignable,
            exclusion_group: def.exclusion_group.clone(),
        }
    }
}

impl From<&rekindle_protocol::dht::community::types::RoleEntryV2> for RoleDto {
    fn from(r: &rekindle_protocol::dht::community::types::RoleEntryV2) -> Self {
        Self {
            id: r.id,
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position,
            hoist: r.hoist,
            mentionable: r.mentionable,
            self_assignable: r.self_assignable,
            exclusion_group: None,
        }
    }
}

// ── Video-cluster event payloads ────────────────────────────────────
//
// Payload structs for the `CommunityEvent` video variants (newtype
// form). Wire-identical to the former inline struct variants under
// `serde(tag = "type", content = "data")` — proven by the snapshot
// tests in `wire_tests.rs`, which pass unchanged across the
// conversion. The frontend TS contract (`src/ipc/channels.ts`) is
// untouched.

use rekindle_video::SessionVideoConfig;

/// Architecture §10.6 — receiver requests an I-frame.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoKeyframeRequestEvent {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub channel_id: String,
    pub stream_id: String,
}

/// Architecture §10.6 — backend-negotiated per-call video config.
/// Recomputed and re-emitted whenever room membership changes or a
/// peer reports new capabilities. Frontend reconciles encoder +
/// decoder by tearing down and reconfiguring with the new
/// constraints. Backend owns the policy — CLI/TUI frontends inherit
/// the same `SessionVideoConfig` payload without re-running any
/// negotiation themselves.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSessionConfigEvent {
    pub community_id: String,
    pub channel_id: String,
    pub config: SessionVideoConfig,
}

/// Phase 3 — the negotiator found NO encoder codec every peer can
/// decode (`negotiate_session_config` returned `None` with a
/// non-empty local encode set). Emitted once per
/// compatible→incompatible transition (latched — membership churn
/// while incompatible does not re-emit). `peers` lists the
/// pseudonyms whose decode sets blocked every local encode codec.
/// Voice is unaffected; the frontend surfaces a toast.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCodecIncompatibleEvent {
    pub community_id: String,
    pub channel_id: String,
    pub peers: Vec<String>,
}

/// Phase 4 — backend bitrate policy output (AIMD over receiver
/// FrameAck/BandwidthEstimate feedback, audio reserve subtracted).
/// The frontend encoder follows this target; the pacer rate moves
/// with it on the backend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoBitrateTargetEvent {
    pub community_id: String,
    pub channel_id: String,
    pub kbps: u32,
}

/// The Linux-native capture session died asynchronously (camera
/// unplugged, pipeline failure) — the panel reverts the camera
/// toggle and surfaces the message. Linux-only: only the native
/// GStreamer pipeline emits this, so the type is gated to where it
/// can be constructed (the webview path on macOS/Windows reports
/// capture failures inline via `setError`, never through a
/// CommunityEvent). The frontend TS union keeps the variant
/// unconditionally — it simply never arrives off-Linux.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeVideoErrorEvent {
    pub community_id: String,
    pub channel_id: String,
    pub message: String,
}

/// Phase F — a gossiped video envelope failed signature or shape
/// verification at the receive boundary. Surfaced to the UI so the
/// asymmetric-drop case (one peer rejects, the other doesn't) is
/// observable from frontend signals instead of grep.
///
/// `communityId` / `senderPseudonym` may be the sentinel string
/// `"<unknown>"` when the envelope failed to deserialize before
/// those routing fields could be read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoEnvelopeRejectedEvent {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub reason: String,
}

/// Architecture §10.6 + Phase 6 Week 22 — the active video relay
/// for a `(channel_id, stream_id)` changed. Frontend should
/// re-attach its decoder to the new relay's stream and discard any
/// partially-buffered frames from the old one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTopologyChangeEvent {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub channel_id: String,
    pub stream_id: String,
    pub relay_host_pseudonym: Option<String>,
    pub reason: String,
    pub lamport: u64,
}
