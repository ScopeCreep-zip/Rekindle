//! Phase 17 — MekRotationEvent emitted to the UI as the rotation
//! protocol progresses. Adapter (src-tauri) maps each variant to a
//! `CommunityEvent::Mek*` shape + base64-encodes any byte fields.

#[derive(Debug, Clone)]
pub enum MekRotationEvent {
    /// The local peer is starting a rotation for `(community, channel)`
    /// at `new_generation`. Either we were elected by the cascade, or
    /// we're rotating proactively (own departure / voice join).
    RotationStarted {
        community_id: String,
        channel_id: String,
        new_generation: u64,
        initiator_pseudonym_hex: String,
    },
    /// The rotation completed locally — every online recipient was
    /// reached (or skipped if offline). Listeners switch the active
    /// MEK to `generation`.
    RotationComplete {
        community_id: String,
        channel_id: String,
        generation: u64,
    },
    /// The rotation failed mid-flight (cascade exhausted, transport
    /// error, etc.). `reason` is a short human-readable string.
    RotationFailed {
        community_id: String,
        channel_id: String,
        reason: String,
    },
    /// We received a wrapped MEK from another peer (the rotator) and
    /// successfully unwrapped + cached it.
    MekDelivered {
        community_id: String,
        channel_id: String,
        generation: u64,
        sender_pseudonym_hex: String,
    },
}
