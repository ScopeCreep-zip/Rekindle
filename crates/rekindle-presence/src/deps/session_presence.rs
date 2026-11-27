//! Our session signals and the voice roster, as the beat path reads them
//! (plan E3.4 S8-pre, decided 2026-11-26): the voice channel we are in,
//! our session and sharing policy (the sealed `extras`), and the voice
//! roster the members' beats feed. The frontend that holds a voice session
//! implements it (the GUI over its voice engine, until E4.1 makes voice the
//! Host's and replaces only the implementation); the registry poll reads
//! the same port until S8b deletes it.

use rekindle_types::presence::{MemberSession, PresenceSharingPolicy, SessionExtras};

pub trait SessionPresence: Send + Sync + 'static {
    /// Voice channel the LOCAL user is connected to in this community, if
    /// any: our beat's `voiceChannel` (MatrixRTC `m.rtc.member` pattern,
    /// readable before the community key converges).
    fn active_voice_channel(&self, community_id: &str) -> Option<String>;

    /// Hand the members' voice membership to the voice layer for roster
    /// reconciliation (add lost-VoiceJoin members, expire ghosts).
    /// Fire-and-forget: the add/remove decisions live in `rekindle-voice`.
    fn reconcile_voice_roster(&self, community_id: &str, rows: Vec<VoicePresenceRow>);

    /// The local user's session in this community: status, focused
    /// channel (`my_session_location`) and last-active.
    fn self_session(&self, community_id: &str) -> MemberSession;

    /// The local user's per-community sharing policy (default-deny).
    /// Local-only, never published: applied before anything is sealed.
    fn presence_policy(&self, community_id: &str) -> PresenceSharingPolicy;
}

/// Presence-derived voice membership view of one community member,
/// handed to the voice roster reconcile from the members' beats (plan E3.4
/// S8-pre) and, until S8b deletes it, the registry scan (architecture
/// three-path: the durable roster; gossip VoiceJoin/VoiceLeave is the fast
/// path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePresenceRow {
    pub pseudonym_hex: String,
    pub display_name: Option<String>,
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
            display_name: None,
            voice_channel_id,
            fresh,
        }
    }
}

/// Our session in `community_id` as our sharing policy lets us publish it
/// (`apply_sharing_policy`, default-deny), with the identity-revealing
/// signals (location, activity) taken out into the extras that are only
/// ever published sealed under the community key; `None` extras when the
/// policy shares neither.
#[must_use]
pub fn shared_session(
    port: &dyn SessionPresence,
    community_id: &str,
) -> (MemberSession, Option<SessionExtras>) {
    let policy = port.presence_policy(community_id);
    let mut session =
        crate::community::policy::apply_sharing_policy(port.self_session(community_id), &policy);
    let extras = SessionExtras {
        location: session.location.take(),
        activity: session.activity.take(),
    };
    let shared = (extras.location.is_some() || extras.activity.is_some()).then_some(extras);
    (session, shared)
}
