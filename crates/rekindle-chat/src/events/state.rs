//! Real-time subscription state — unread counts, typing indicators,
//! presence dots, voice participant lists.
//!
//! Reactive client-side state maintained from inbound network events.
//! Every inbound event updates this state, and the TUI reads it on
//! every render frame — no user action required for live updates.
//!
//! All state is behind `parking_lot::RwLock` in ChatService. Readers
//! (TUI badge queries) take a read lock. Writers (event handlers) take
//! a write lock. Ephemeral state (typing, presence) auto-expires on
//! read — no background cleanup task needed.

use std::collections::HashMap;
use std::time::Instant;

/// Combined subscription state. One instance per ChatService.
#[derive(Debug, Default)]
pub struct SubscriptionState {
    pub unread: UnreadState,
    pub typing: TypingState,
    pub presence: PresenceState,
    pub voice: VoiceState,
}

// ── Unread tracking ──────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct UnreadState {
    pub channels: HashMap<(String, String), u32>,
    pub dms: HashMap<String, u32>,
    pub friend_requests: u32,
}

impl UnreadState {
    pub fn increment_channel(&mut self, community: &str, channel: &str) -> u32 {
        let key = (community.to_string(), channel.to_string());
        let count = self.channels.entry(key).or_insert(0);
        *count = count.saturating_add(1);
        *count
    }

    pub fn increment_dm(&mut self, peer_key: &str) -> u32 {
        let count = self.dms.entry(peer_key.to_string()).or_insert(0);
        *count = count.saturating_add(1);
        *count
    }

    pub fn mark_channel_read(&mut self, community: &str, channel: &str) -> u32 {
        self.channels.remove(&(community.to_string(), channel.to_string())).unwrap_or(0)
    }

    pub fn mark_dm_read(&mut self, peer_key: &str) -> u32 {
        self.dms.remove(peer_key).unwrap_or(0)
    }

    pub fn remove_community(&mut self, community: &str) {
        self.channels.retain(|k, _| k.0 != community);
    }

    pub fn remove_dm_peer(&mut self, peer_key: &str) {
        self.dms.remove(peer_key);
    }
}

// ── Typing indicators ────────────────────────────────────────────────

const TYPING_EXPIRY_SECS: u64 = 5;

#[derive(Debug, Default)]
pub struct TypingState {
    pub channels: HashMap<(String, String), Vec<TypingEntry>>,
    pub dms: HashMap<String, Instant>,
}

#[derive(Debug, Clone)]
pub struct TypingEntry {
    pub pseudonym: String,
    pub last_seen: Instant,
}

impl TypingState {
    pub fn set_channel_typing(&mut self, community: &str, channel: &str, pseudonym: &str) -> bool {
        let key = (community.to_string(), channel.to_string());
        let entries = self.channels.entry(key).or_default();
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS))
            .expect("typing expiry within uptime");
        entries.retain(|e| e.last_seen > cutoff);

        if let Some(entry) = entries.iter_mut().find(|e| e.pseudonym == pseudonym) {
            entry.last_seen = Instant::now();
            false
        } else {
            entries.push(TypingEntry { pseudonym: pseudonym.to_string(), last_seen: Instant::now() });
            true
        }
    }

    pub fn channel_typers(&mut self, community: &str, channel: &str) -> Vec<String> {
        let key = (community.to_string(), channel.to_string());
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS))
            .expect("typing expiry within uptime");
        if let Some(entries) = self.channels.get_mut(&key) {
            entries.retain(|e| e.last_seen > cutoff);
            entries.iter().map(|e| e.pseudonym.clone()).collect()
        } else {
            Vec::new()
        }
    }

    pub fn collect_expired_channel_typers(&mut self) -> Vec<(String, String, String)> {
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS))
            .expect("typing expiry within uptime");
        let mut expired = Vec::new();
        for ((community, channel), entries) in &mut self.channels {
            for entry in entries.iter() {
                if entry.last_seen <= cutoff {
                    expired.push((community.clone(), channel.clone(), entry.pseudonym.clone()));
                }
            }
            entries.retain(|e| e.last_seen > cutoff);
        }
        self.channels.retain(|_, entries| !entries.is_empty());
        expired
    }

    pub fn set_dm_typing(&mut self, peer_key: &str) -> bool {
        let is_new = !self.dms.contains_key(peer_key);
        self.dms.insert(peer_key.to_string(), Instant::now());
        is_new
    }

    pub fn is_dm_typing(&self, peer_key: &str) -> bool {
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS))
            .expect("typing expiry within uptime");
        self.dms.get(peer_key).is_some_and(|ts| *ts > cutoff)
    }

    pub fn collect_expired_dm_typers(&mut self) -> Vec<String> {
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(TYPING_EXPIRY_SECS))
            .expect("typing expiry within uptime");
        let expired: Vec<String> = self.dms.iter()
            .filter(|(_, ts)| **ts <= cutoff)
            .map(|(k, _)| k.clone())
            .collect();
        for key in &expired { self.dms.remove(key); }
        expired
    }

    pub fn remove_community(&mut self, community: &str) {
        self.channels.retain(|k, _| k.0 != community);
    }

    pub fn remove_dm_peer(&mut self, peer_key: &str) {
        self.dms.remove(peer_key);
    }
}

// ── Presence tracking ────────────────────────────────────────────────

const PRESENCE_EXPIRY_SECS: u64 = 300;
const MAX_PRESENCE_PER_COMMUNITY: usize = 10_000;

#[derive(Debug, Clone)]
pub struct PresenceInfo {
    pub status: String,
    pub game_name: Option<String>,
    pub game_id: Option<u32>,
    pub last_seen: Instant,
}

impl PresenceInfo {
    pub fn is_expired(&self) -> bool {
        self.last_seen.elapsed() > std::time::Duration::from_secs(PRESENCE_EXPIRY_SECS)
    }

    pub fn effective_status(&self) -> &str {
        if self.is_expired() { "offline" } else { &self.status }
    }
}

#[derive(Debug, Default)]
pub struct PresenceState {
    pub members: HashMap<(String, String), PresenceInfo>,
    pub friends: HashMap<String, PresenceInfo>,
}

impl PresenceState {
    pub fn set_member(
        &mut self, community: &str, pseudonym: &str,
        status: &str, game_name: Option<&str>, game_id: Option<u32>,
    ) {
        let key = (community.to_string(), pseudonym.to_string());
        if !self.members.contains_key(&key) {
            let community_count = self.members.keys().filter(|(c, _)| c == community).count();
            if community_count >= MAX_PRESENCE_PER_COMMUNITY {
                tracing::debug!(community, pseudonym = &pseudonym[..12.min(pseudonym.len())], "presence cap reached");
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

    pub fn set_friend(&mut self, peer_key: &str, status: &str, game_name: Option<&str>) {
        self.friends.insert(peer_key.to_string(), PresenceInfo {
            status: status.to_string(),
            game_name: game_name.map(String::from),
            game_id: None,
            last_seen: Instant::now(),
        });
    }

    pub fn community_members(&self, community: &str) -> Vec<(String, PresenceInfo)> {
        self.members.iter()
            .filter(|((c, _), _)| c == community)
            .map(|((_, p), info)| (p.clone(), info.clone()))
            .collect()
    }

    pub fn friend(&self, peer_key: &str) -> Option<&PresenceInfo> {
        self.friends.get(peer_key).filter(|p| !p.is_expired())
    }

    pub fn remove_community(&mut self, community: &str) {
        self.members.retain(|k, _| k.0 != community);
    }

    pub fn remove_dm_peer(&mut self, peer_key: &str) {
        self.friends.remove(peer_key);
    }
}

// ── Voice state ──────────────────────────────────────────────────────

const VOICE_EXPIRY_SECS: u64 = 60;

#[derive(Debug, Clone)]
pub struct VoiceParticipantInfo {
    pub pseudonym_key: String,
    pub muted: bool,
    pub deafened: bool,
    pub joined_at: u64,
    pub last_heartbeat: Instant,
}

#[derive(Debug, Default)]
pub struct VoiceState {
    pub channels: HashMap<(String, String), Vec<VoiceParticipantInfo>>,
}

impl VoiceState {
    pub fn join(&mut self, community: &str, channel: &str, pseudonym: &str, timestamp: u64) {
        let key = (community.to_string(), channel.to_string());
        let participants = self.channels.entry(key).or_default();
        if let Some(p) = participants.iter_mut().find(|p| p.pseudonym_key == pseudonym) {
            p.last_heartbeat = Instant::now();
        } else {
            participants.push(VoiceParticipantInfo {
                pseudonym_key: pseudonym.to_string(),
                muted: false, deafened: false,
                joined_at: timestamp,
                last_heartbeat: Instant::now(),
            });
        }
    }

    pub fn leave(&mut self, community: &str, channel: &str, pseudonym: &str) {
        let key = (community.to_string(), channel.to_string());
        if let Some(participants) = self.channels.get_mut(&key) {
            participants.retain(|p| p.pseudonym_key != pseudonym);
            if participants.is_empty() { self.channels.remove(&key); }
        }
    }

    pub fn participants(&mut self, community: &str, channel: &str) -> Vec<VoiceParticipantInfo> {
        let key = (community.to_string(), channel.to_string());
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(VOICE_EXPIRY_SECS))
            .expect("voice expiry within uptime");
        if let Some(participants) = self.channels.get_mut(&key) {
            participants.retain(|p| p.last_heartbeat > cutoff);
            participants.clone()
        } else {
            Vec::new()
        }
    }

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

    pub fn remove_community(&mut self, community: &str) {
        self.channels.retain(|k, _| k.0 != community);
    }
}
