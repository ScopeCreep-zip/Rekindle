use serde::Serialize;

/// System-level notification events.
///
/// `MessageReceived` carries the resolved per-channel/per-community
/// `sound_ref` (architecture §32 Phase 7 Week 25) so the frontend can
/// pick the right notification sound without an extra round-trip.
/// `SystemAlert` is for app-level events (network connect, decrypt
/// failure, etc.) that don't belong to a specific channel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum NotificationEvent {
    MessageReceived {
        title: String,
        body: String,
        community_id: String,
        channel_id: String,
        /// Resolved via channel override → community default →
        /// `None` (frontend uses bundled default).
        sound_ref: Option<String>,
    },
    SystemAlert {
        title: String,
        body: String,
    },
    UpdateAvailable {
        version: String,
    },
    /// P3.3 session renewal — peer sent a SessionResetRequest. Frontend
    /// surfaces a confirmation modal showing the peer's display name +
    /// safety number; user must verify out-of-band before accepting.
    /// Carries the safety_number (BLAKE3 of sorted (our_identity_key,
    /// peer_identity_key) — same on both sides) so the user can compare
    /// against the value the peer reads from their UI.
    #[serde(rename_all = "camelCase")]
    SessionResetRequested {
        peer_public_key: String,
        peer_display_name: String,
        /// Hex-encoded short safety number (8 hex chars = 32 bits) for
        /// out-of-band comparison. Bigger than a phone number but small
        /// enough to read aloud.
        safety_number: String,
    },
}

/// Pushed to the frontend whenever network-relevant state changes
/// (attachment, readiness, or route allocation) so the `NetworkIndicator`
/// can update instantly instead of polling.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatusEvent {
    /// Raw Veilid `AttachmentState` string (e.g. "detached", "attaching", "`attached_good`").
    pub attachment_state: String,
    pub is_attached: bool,
    pub public_internet_ready: bool,
    /// Whether we have an allocated private route for receiving messages.
    pub has_route: bool,
}
