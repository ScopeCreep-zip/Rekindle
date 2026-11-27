//! The shell's voice state, as the beat path reads it (plan E3.4 S8-pre,
//! ruled by the lead 2026-11-26 on `58eb04bd7`): the voice channel we are
//! in and the voice roster the members' beats feed, and nothing else. Our
//! session and sharing policy are the Host's own state, never the shell's.
//! The frontend that holds a voice session implements it (the GUI over its
//! voice engine) until E4.1 makes voice the Host's and deletes it; the
//! registry poll reads the same port until S8b deletes the poll.

pub trait SessionPresence: Send + Sync + 'static {
    /// Voice channel the LOCAL user is connected to in this community, if
    /// any: our beat's `voiceChannel` (MatrixRTC `m.rtc.member` pattern,
    /// readable before the community key converges).
    fn active_voice_channel(&self, community_id: &str) -> Option<String>;

    /// Hand the members' voice membership to the voice layer for roster
    /// reconciliation (add lost-VoiceJoin members, expire ghosts).
    /// Fire-and-forget: the add/remove decisions live in `rekindle-voice`.
    fn reconcile_voice_roster(&self, community_id: &str, rows: Vec<VoicePresenceRow>);
}

/// Presence-derived voice membership view of one community member,
/// handed to the voice roster reconcile from the members' beats (plan E3.4
/// S8-pre) and, until S8b deletes it, the registry scan (architecture
/// three-path: the durable roster; gossip VoiceJoin/VoiceLeave is the fast
/// path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePresenceRow {
    pub pseudonym_hex: String,
    /// The cleartext voice channel claim on the row. The row carries no
    /// media route (plan C7.15): the reconcile introduces us through the
    /// call's signaling instead.
    pub voice_channel_id: Option<String>,
    /// Row passed the scan's liveness gate (fresh heartbeat +
    /// non-offline). Stale rows still flow through so the reconcile
    /// can expire ghosts.
    pub fresh: bool,
}

impl VoicePresenceRow {
    /// `pseudonym_hex`'s claim to be in `voice_channel_id`; `fresh` when
    /// its presence is live. No name: a member's name is its card's (plan
    /// E3.4 S6).
    #[must_use]
    pub fn claim(pseudonym_hex: String, voice_channel_id: Option<String>, fresh: bool) -> Self {
        Self {
            pseudonym_hex,
            voice_channel_id,
            fresh,
        }
    }
}
