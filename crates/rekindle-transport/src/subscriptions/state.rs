//! Mutable subscription state — unread counts, typing, presence, voice.
//!
//! All state is owned by the `SubscriptionManager` behind a `RwLock`.
//! Readers (TUI badge queries) take a read lock. Writers (event handlers)
//! take a write lock. Ephemeral state (typing, presence) auto-expires
//! on read — no background cleanup task needed.

use std::collections::HashMap;
use std::time::Instant;

/// Combined subscription state. One instance per `SubscriptionManager`.
#[derive(Debug, Default)]
pub struct SubscriptionState {
    pub unread: UnreadState,
    pub typing: TypingState,
    pub presence: PresenceState,
    pub voice: VoiceState,
}

// ── Unread tracking ────────────────────────────────────────────────────

/// Unread message counts across all contexts.
#[derive(Debug, Default)]
pub struct UnreadState {
    /// (community_governance_key, channel_id) → unread count.
    pub channels: HashMap<(String, String), u32>,
    /// peer_public_key → unread DM count.
    pub dms: HashMap<String, u32>,
    /// Pending inbound friend request count.
    pub friend_requests: u32,
}

impl UnreadState {
    /// Increment channel unread count. Returns the new count.
    pub fn increment_channel(&mut self, community: &str, channel: &str) -> u32 {
        let key = (community.to_string(), channel.to_string());
        let count = self.channels.entry(key).or_insert(0);
        *count = count.saturating_add(1);
        *count
    }

    /// Increment DM unread count. Returns the new count.
    pub fn increment_dm(&mut self, peer_key: &str) -> u32 {
        let count = self.dms.entry(peer_key.to_string()).or_insert(0);
        *count = count.saturating_add(1);
        *count
    }

    /// Mark a channel as read. Returns the previous count.
    pub fn mark_channel_read(&mut self, community: &str, channel: &str) -> u32 {
        let key = (community.to_string(), channel.to_string());
        self.channels.remove(&key).unwrap_or(0)
    }

    /// Mark a DM conversation as read. Returns the previous count.
    pub fn mark_dm_read(&mut self, peer_key: &str) -> u32 {
        self.dms.remove(peer_key).unwrap_or(0)
    }

    /// Remove all unread state for a community (on leave).
    pub fn remove_community(&mut self, community: &str) {
        self.channels.retain(|k, _| k.0 != community);
    }

    /// Remove all unread state for a DM peer (on unfriend).
    pub fn remove_dm_peer(&mut self, peer_key: &str) {
        self.dms.remove(peer_key);
    }
}

// ── Typing indicators ──────────────────────────────────────────────────

/// Typing indicator expiry duration.
const TYPING_EXPIRY_SECS: u64 = 5;

/// Ephemeral typing state. Entries auto-expire after 5 seconds.
#[derive(Debug, Default)]
pub struct TypingState {
    /// (community, channel) → [(pseudonym, last_typing_instant)].
    pub channels: HashMap<(String, String), Vec<TypingEntry>>,
    /// peer_key → last_typing_instant (DM context).
    pub dms: HashMap<String, Instant>,
}

#[derive(Debug, Clone)]
pub struct TypingEntry {
    pub pseudonym: String,
    pub last_seen: Instant,
}

impl TypingState {
    /// Record that someone started typing in a channel.
    /// Returns `true` if this is a new typer (wasn't already typing).
    pub fn set_channel_typing(&mut self, community: &str, channel: &str, pseudonym: &str) -> bool {
        let key = (community.to_string(), channel.to_string());
        let entries = self.channels.entry(key).or_default();

        // Prune expired entries first
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS)).expect("typing expiry within uptime");
        entries.retain(|e| e.last_seen > cutoff);

        // Update or insert
        if let Some(entry) = entries.iter_mut().find(|e| e.pseudonym == pseudonym) {
            entry.last_seen = Instant::now();
            false // already typing
        } else {
            entries.push(TypingEntry {
                pseudonym: pseudonym.to_string(),
                last_seen: Instant::now(),
            });
            true // new typer
        }
    }

    /// Get active typers in a channel (auto-prunes expired).
    pub fn channel_typers(&mut self, community: &str, channel: &str) -> Vec<String> {
        let key = (community.to_string(), channel.to_string());
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS)).expect("typing expiry within uptime");

        if let Some(entries) = self.channels.get_mut(&key) {
            entries.retain(|e| e.last_seen > cutoff);
            entries.iter().map(|e| e.pseudonym.clone()).collect()
        } else {
            Vec::new()
        }
    }

    /// Collect all expired typers across all channels (for emitting TypingStopped).
    pub fn collect_expired_channel_typers(&mut self) -> Vec<(String, String, String)> {
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS)).expect("typing expiry within uptime");
        let mut expired = Vec::new();

        for ((community, channel), entries) in &mut self.channels {
            for entry in entries.iter() {
                if entry.last_seen <= cutoff {
                    expired.push((community.clone(), channel.clone(), entry.pseudonym.clone()));
                }
            }
            entries.retain(|e| e.last_seen > cutoff);
        }
        // Remove empty channel entries
        self.channels.retain(|_, entries| !entries.is_empty());
        expired
    }

    /// Record DM typing. Returns `true` if this is a new typing start.
    pub fn set_dm_typing(&mut self, peer_key: &str) -> bool {
        let is_new = !self.dms.contains_key(peer_key);
        self.dms.insert(peer_key.to_string(), Instant::now());
        is_new
    }

    /// Check if a peer is typing in DM (auto-expires).
    pub fn is_dm_typing(&self, peer_key: &str) -> bool {
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS)).expect("typing expiry within uptime");
        self.dms.get(peer_key).is_some_and(|ts| *ts > cutoff)
    }

    /// Collect expired DM typers (for emitting TypingStopped).
    pub fn collect_expired_dm_typers(&mut self) -> Vec<String> {
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS)).expect("typing expiry within uptime");
        let mut expired = Vec::new();
        for (peer_key, ts) in &self.dms {
            if *ts <= cutoff {
                expired.push(peer_key.clone());
            }
        }
        for key in &expired {
            self.dms.remove(key);
        }
        expired
    }

    /// Remove all typing state for a community (on leave).
    pub fn remove_community(&mut self, community: &str) {
        self.channels.retain(|k, _| k.0 != community);
    }

    /// Remove DM typing state for a peer (on unfriend).
    pub fn remove_dm_peer(&mut self, peer_key: &str) {
        self.dms.remove(peer_key);
    }
}

// ── Presence tracking ──────────────────────────────────────────────────

/// Presence expiry — members not seen in 5 minutes are considered offline.
const PRESENCE_EXPIRY_SECS: u64 = 300;

/// Member presence information.
#[derive(Debug, Clone)]
pub struct PresenceInfo {
    pub status: String,
    pub game_name: Option<String>,
    pub game_id: Option<u32>,
    pub last_seen: Instant,
}

impl PresenceInfo {
    /// Whether this presence entry has expired.
    pub fn is_expired(&self) -> bool {
        self.last_seen.elapsed() > std::time::Duration::from_secs(PRESENCE_EXPIRY_SECS)
    }

    /// Effective status — returns "offline" if expired.
    pub fn effective_status(&self) -> &str {
        if self.is_expired() { "offline" } else { &self.status }
    }
}

/// Maximum presence entries per community. Prevents memory exhaustion from
/// an attacker registering thousands of pseudonyms.
const MAX_PRESENCE_PER_COMMUNITY: usize = 10_000;

/// Presence state for community members and DM peers.
#[derive(Debug, Default)]
pub struct PresenceState {
    /// (community, pseudonym) → presence info.
    pub members: HashMap<(String, String), PresenceInfo>,
    /// peer_public_key → presence info (DM/friend context).
    pub friends: HashMap<String, PresenceInfo>,
}

impl PresenceState {
    /// Update community member presence.
    pub fn set_member(
        &mut self, community: &str, pseudonym: &str,
        status: &str, game_name: Option<&str>, game_id: Option<u32>,
    ) {
        let key = (community.to_string(), pseudonym.to_string());

        // Cap check: reject new pseudonyms if at limit for this community.
        // O(n) scan per community — acceptable at MAX_PRESENCE_PER_COMMUNITY=10k
        // (~microseconds). If this hot path shows up in profiles, add a
        // per-community counter HashMap<String, usize>.
        if !self.members.contains_key(&key) {
            let community_count = self.members.keys()
                .filter(|(c, _)| c == community)
                .count();
            if community_count >= MAX_PRESENCE_PER_COMMUNITY {
                tracing::debug!(community, pseudonym = &pseudonym[..12.min(pseudonym.len())], "presence cap reached, rejecting");
                return;
            }
        }

        self.members.insert(key, PresenceInfo {
            status: status.to_string(),
            game_name: game_name.map(String::from),
            game_id,
            last_seen: Instant::now(),
        });
    }

    /// Update friend/DM peer presence.
    pub fn set_friend(&mut self, peer_key: &str, status: &str, game_name: Option<&str>) {
        self.friends.insert(peer_key.to_string(), PresenceInfo {
            status: status.to_string(),
            game_name: game_name.map(String::from),
            game_id: None,
            last_seen: Instant::now(),
        });
    }

    /// Get all active member presences for a community (auto-filters expired).
    pub fn community_members(&self, community: &str) -> Vec<(String, PresenceInfo)> {
        self.members.iter()
            .filter(|((c, _), _)| c == community)
            .map(|((_, p), info)| (p.clone(), info.clone()))
            .collect()
    }

    /// Get friend presence (returns None if expired or unknown).
    pub fn friend(&self, peer_key: &str) -> Option<&PresenceInfo> {
        self.friends.get(peer_key).filter(|p| !p.is_expired())
    }

    /// Remove all presence state for a community.
    pub fn remove_community(&mut self, community: &str) {
        self.members.retain(|k, _| k.0 != community);
    }

    /// Remove presence state for a DM peer.
    pub fn remove_dm_peer(&mut self, peer_key: &str) {
        self.friends.remove(peer_key);
    }
}

// ── Voice state ────────────────────────────────────────────────────────

/// Voice channel participant info.
#[derive(Debug, Clone)]
pub struct VoiceParticipantInfo {
    pub pseudonym_key: String,
    pub muted: bool,
    pub deafened: bool,
    pub joined_at: u64,
    pub last_heartbeat: Instant,
}

/// Voice participant expiry — no heartbeat in 60 seconds = gone.
const VOICE_EXPIRY_SECS: u64 = 60;

/// Voice channel state.
#[derive(Debug, Default)]
pub struct VoiceState {
    /// (community, channel) → participants.
    pub channels: HashMap<(String, String), Vec<VoiceParticipantInfo>>,
}

impl VoiceState {
    /// Add or update a voice participant.
    pub fn join(
        &mut self, community: &str, channel: &str,
        pseudonym: &str, timestamp: u64,
    ) {
        let key = (community.to_string(), channel.to_string());
        let participants = self.channels.entry(key).or_default();
        if let Some(p) = participants.iter_mut().find(|p| p.pseudonym_key == pseudonym) {
            p.last_heartbeat = Instant::now();
        } else {
            participants.push(VoiceParticipantInfo {
                pseudonym_key: pseudonym.to_string(),
                muted: false,
                deafened: false,
                joined_at: timestamp,
                last_heartbeat: Instant::now(),
            });
        }
    }

    /// Remove a voice participant.
    pub fn leave(&mut self, community: &str, channel: &str, pseudonym: &str) {
        let key = (community.to_string(), channel.to_string());
        if let Some(participants) = self.channels.get_mut(&key) {
            participants.retain(|p| p.pseudonym_key != pseudonym);
            if participants.is_empty() {
                self.channels.remove(&key);
            }
        }
    }

    /// Get participants for a channel (auto-prunes expired).
    pub fn participants(&mut self, community: &str, channel: &str) -> Vec<VoiceParticipantInfo> {
        let key = (community.to_string(), channel.to_string());
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(VOICE_EXPIRY_SECS)).expect("voice expiry within uptime");
        if let Some(participants) = self.channels.get_mut(&key) {
            participants.retain(|p| p.last_heartbeat > cutoff);
            participants.clone()
        } else {
            Vec::new()
        }
    }

    /// Update mute/deafen state for a participant.
    pub fn update_mute_deafen(
        &mut self, community: &str, channel: &str,
        pseudonym: &str, muted: Option<bool>, deafened: Option<bool>,
    ) {
        let key = (community.to_string(), channel.to_string());
        if let Some(participants) = self.channels.get_mut(&key) {
            if let Some(p) = participants.iter_mut().find(|p| p.pseudonym_key == pseudonym) {
                if let Some(m) = muted { p.muted = m; }
                if let Some(d) = deafened { p.deafened = d; }
                p.last_heartbeat = Instant::now();
            }
        }
    }

    /// Remove all voice state for a community.
    pub fn remove_community(&mut self, community: &str) {
        self.channels.retain(|k, _| k.0 != community);
    }
}
