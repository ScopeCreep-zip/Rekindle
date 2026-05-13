//! Cross-tier event deduplication via BLAKE3 content hashing.
//!
//! Every `SubscriptionEvent` is BLAKE3-hashed before emission to the IPC bus.
//! The hash covers semantically meaningful fields — not pathway-specific metadata
//! like arrival timestamp or source tier. A single bounded `HashSet<[u8; 32]>`
//! rejects duplicates regardless of whether the event arrived via DHT watch,
//! gossip mesh, or periodic poll.
//!
//! At 100K+ agent scale, this ensures the IPC bus carries only unique events.
//! The same logical event may arrive up to 3 times (watch + gossip + poll) —
//! only the first is emitted.

use std::collections::{HashSet, VecDeque};
use std::time::Instant;

use tracing::{debug, trace};

use rekindle_types::subscription_events::{
    SubscriptionEvent,
    ChannelMessageEvent, TypingEvent, TypingContext,
    PresenceEvent, MembershipEvent, FriendEvent,
    CryptoEvent, VoiceEvent, GovernanceEvent,
    SocialEvent, NetworkEvent, SystemEvent,
};

/// Content-addressed event deduplication using BLAKE3 digests.
///
/// Events are hashed by their canonical content (not pathway metadata).
/// The digest set is bounded by capacity with FIFO eviction. Expired
/// entries are also evicted on periodic cleanup.
pub struct EventDedup {
    digests: HashSet<[u8; 32]>,
    order: VecDeque<([u8; 32], Instant)>,
    capacity: usize,
    ttl_secs: u64,
    suppressed_count: u64,
}

impl EventDedup {
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            digests: HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
            ttl_secs,
            suppressed_count: 0,
        }
    }

    /// Check if this event is new (should emit) or duplicate (suppress).
    ///
    /// Returns `true` if the event is new and has been recorded.
    /// Returns `false` if the event is a duplicate and should be dropped.
    pub fn check(&mut self, event: &SubscriptionEvent) -> bool {
        // UnreadChanged is always emitted — it's a computed aggregate, not a network event
        if matches!(event, SubscriptionEvent::UnreadChanged { .. }) {
            return true;
        }

        let digest = hash_event(event);

        if self.digests.contains(&digest) {
            self.suppressed_count += 1;
            trace!(suppressed = self.suppressed_count, "dedup: duplicate suppressed");
            return false;
        }

        if self.digests.len() >= self.capacity {
            if let Some((old_digest, _)) = self.order.pop_front() {
                self.digests.remove(&old_digest);
            }
        }

        self.digests.insert(digest);
        self.order.push_back((digest, Instant::now()));
        true
    }

    /// Evict entries older than TTL.
    pub fn evict_expired(&mut self) {
        let cutoff = self.ttl_secs;
        let mut evicted = 0u32;
        while let Some((digest, inserted)) = self.order.front() {
            if inserted.elapsed().as_secs() > cutoff {
                let d = *digest;
                self.order.pop_front();
                self.digests.remove(&d);
                evicted += 1;
            } else {
                break;
            }
        }
        if evicted > 0 {
            debug!(evicted, remaining = self.digests.len(), "dedup: expired entries evicted");
        }
    }

    pub fn len(&self) -> usize { self.digests.len() }
    pub fn is_empty(&self) -> bool { self.digests.is_empty() }
    pub fn suppressed_count(&self) -> u64 { self.suppressed_count }
}

impl Default for EventDedup {
    fn default() -> Self {
        Self::new(10_000, 300) // 10K entries, 5-minute TTL
    }
}

// ── BLAKE3 content hashing ───────────────────────────────────────────

trait HashField { fn hash_into(&self, h: &mut blake3::Hasher); }
impl HashField for str    { fn hash_into(&self, h: &mut blake3::Hasher) { h.update(self.as_bytes()); } }
impl HashField for String { fn hash_into(&self, h: &mut blake3::Hasher) { h.update(self.as_bytes()); } }
impl HashField for u32    { fn hash_into(&self, h: &mut blake3::Hasher) { h.update(&self.to_le_bytes()); } }
impl HashField for u64    { fn hash_into(&self, h: &mut blake3::Hasher) { h.update(&self.to_le_bytes()); } }
impl HashField for bool   { fn hash_into(&self, h: &mut blake3::Hasher) { h.update(&[u8::from(*self)]); } }
impl HashField for usize  { fn hash_into(&self, h: &mut blake3::Hasher) { h.update(&(*self as u64).to_le_bytes()); } }
impl<T: HashField + ?Sized> HashField for &T { fn hash_into(&self, h: &mut blake3::Hasher) { (**self).hash_into(h); } }

macro_rules! hash_fields {
    ($h:expr, $tag:expr $(, $field:expr)* $(,)?) => {{
        $h.update($tag.as_bytes());
        $( $h.update(b"|"); HashField::hash_into(&$field, $h); )*
    }};
}

fn hash_event(event: &SubscriptionEvent) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(event_tag(event).as_bytes());
    h.update(b"|");
    match event {
        SubscriptionEvent::ChannelMessage(m) => hash_channel_message(&mut h, m),
        SubscriptionEvent::Typing(t) => hash_typing(&mut h, t),
        SubscriptionEvent::Presence(p) => hash_presence(&mut h, p),
        SubscriptionEvent::Membership(m) => hash_membership(&mut h, m),
        SubscriptionEvent::Friend(f) => hash_friend(&mut h, f),
        SubscriptionEvent::Crypto(c) => hash_crypto(&mut h, c),
        SubscriptionEvent::Voice(v) => hash_voice(&mut h, v),
        SubscriptionEvent::Governance(g) => hash_governance(&mut h, g),
        SubscriptionEvent::Social(s) => hash_social(&mut h, s),
        SubscriptionEvent::Network(n) => hash_network(&mut h, n),
        SubscriptionEvent::System(s) => hash_system(&mut h, s),
        SubscriptionEvent::UnreadChanged { .. } => { h.update(b"unread"); }
        SubscriptionEvent::BulkTransferProgress { transfer_id, bytes_transferred, .. } => {
            hash_fields!(&mut h, "bulk_prog", transfer_id, *bytes_transferred);
        }
    }
    *h.finalize().as_bytes()
}

fn event_tag(event: &SubscriptionEvent) -> &'static str {
    match event {
        SubscriptionEvent::ChannelMessage(_) => "ch",
        SubscriptionEvent::Typing(_) => "ty",
        SubscriptionEvent::Presence(_) => "pr",
        SubscriptionEvent::Membership(_) => "mb",
        SubscriptionEvent::Friend(_) => "fr",
        SubscriptionEvent::Crypto(_) => "cr",
        SubscriptionEvent::Voice(_) => "vo",
        SubscriptionEvent::Governance(_) => "go",
        SubscriptionEvent::Social(_) => "so",
        SubscriptionEvent::Network(_) => "ne",
        SubscriptionEvent::System(_) => "sy",
        SubscriptionEvent::UnreadChanged { .. } => "ur",
        SubscriptionEvent::BulkTransferProgress { .. } => "bt",
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hash_channel_message(h: &mut blake3::Hasher, msg: &ChannelMessageEvent) {
    match msg {
        ChannelMessageEvent::New { community, channel, message_id, .. } =>
            hash_fields!(h, "new", community, channel, message_id),
        ChannelMessageEvent::Edited { community, channel, message_id, .. } =>
            hash_fields!(h, "edited", community, channel, message_id),
        ChannelMessageEvent::Deleted { community, channel, message_id } =>
            hash_fields!(h, "deleted", community, channel, message_id),
        ChannelMessageEvent::DirectMessageReceived { peer_key, timestamp, .. } =>
            hash_fields!(h, "dm", peer_key, timestamp / 1000),
    }
}

fn hash_typing(h: &mut blake3::Hasher, t: &TypingEvent) {
    let bucket = now_secs() / 5;
    match t {
        TypingEvent::Started { context, who } => {
            hash_fields!(h, "start");
            hash_typing_context(h, context);
            hash_fields!(h, "", who, bucket);
        }
        TypingEvent::Stopped { context, who } => {
            hash_fields!(h, "stop");
            hash_typing_context(h, context);
            hash_fields!(h, "", who, bucket);
        }
    }
}

fn hash_typing_context(h: &mut blake3::Hasher, ctx: &TypingContext) {
    match ctx {
        TypingContext::Channel { community, channel } => hash_fields!(h, "ch", community, channel),
        TypingContext::Dm { peer_key } => hash_fields!(h, "dm", peer_key),
    }
}

fn hash_presence(h: &mut blake3::Hasher, p: &PresenceEvent) {
    let bucket = now_secs() / 30;
    match p {
        PresenceEvent::CommunityMemberChanged { community, pseudonym, status, .. } =>
            hash_fields!(h, "community", community, pseudonym, status, bucket),
        PresenceEvent::FriendChanged { peer_key, status, .. } =>
            hash_fields!(h, "friend", peer_key, status, bucket),
    }
}

fn hash_membership(h: &mut blake3::Hasher, m: &MembershipEvent) {
    match m {
        MembershipEvent::JoinRequested { community, pseudonym, .. } => hash_fields!(h, "join_req", community, pseudonym),
        MembershipEvent::JoinAccepted { community, .. } => hash_fields!(h, "join_acc", community),
        MembershipEvent::JoinRejected { community, reason } => hash_fields!(h, "join_rej", community, reason),
        MembershipEvent::Joined { community, pseudonym, .. } => hash_fields!(h, "joined", community, pseudonym),
        MembershipEvent::Left { community, pseudonym } => hash_fields!(h, "left", community, pseudonym),
        MembershipEvent::Removed { community, pseudonym } => hash_fields!(h, "removed", community, pseudonym),
        MembershipEvent::Kicked { community, target_pseudonym } => hash_fields!(h, "kicked", community, target_pseudonym),
        MembershipEvent::Banned { community, target_pseudonym } => hash_fields!(h, "banned", community, target_pseudonym),
        MembershipEvent::Unbanned { community, target_pseudonym } => hash_fields!(h, "unbanned", community, target_pseudonym),
        MembershipEvent::TimedOut { community, target_pseudonym, .. } => hash_fields!(h, "timeout", community, target_pseudonym),
        MembershipEvent::TimeoutRemoved { community, target_pseudonym } => hash_fields!(h, "timeout_rm", community, target_pseudonym),
        MembershipEvent::TimeoutStatusChanged { community, pseudonym, .. } => hash_fields!(h, "timeout_st", community, pseudonym),
        MembershipEvent::RolesChanged { community, pseudonym, role_ids } => {
            hash_fields!(h, "roles", community, pseudonym);
            for id in role_ids { h.update(&id.to_le_bytes()); }
        }
        MembershipEvent::OnboardingCompleted { community, pseudonym, .. } => hash_fields!(h, "onboard", community, pseudonym),
        MembershipEvent::OnboardingAnswersSubmitted { community, sender_pseudonym, .. } => hash_fields!(h, "onboard_ans", community, sender_pseudonym),
    }
}

fn hash_friend(h: &mut blake3::Hasher, f: &FriendEvent) {
    match f {
        FriendEvent::RequestReceived { from_key, .. } => hash_fields!(h, "req", from_key),
        FriendEvent::RequestAcknowledged { peer_key } => hash_fields!(h, "ack", peer_key),
        FriendEvent::Accepted { peer_key, .. } => hash_fields!(h, "acc", peer_key),
        FriendEvent::Rejected { peer_key } => hash_fields!(h, "rej", peer_key),
        FriendEvent::Removed { peer_key } => hash_fields!(h, "rem", peer_key),
        FriendEvent::RemoveAcknowledged { peer_key } => hash_fields!(h, "rem_ack", peer_key),
        FriendEvent::ProfileKeyRotated { peer_key, new_profile_dht_key } => hash_fields!(h, "rot", peer_key, new_profile_dht_key),
    }
}

fn hash_crypto(h: &mut blake3::Hasher, c: &CryptoEvent) {
    match c {
        CryptoEvent::MekRotated { community, channel, generation, .. } =>
            hash_fields!(h, "mek_rot", community, channel.as_deref().unwrap_or("all"), *generation),
        CryptoEvent::MekRequested { community, channel, needed_generation, requester_pseudonym } =>
            hash_fields!(h, "mek_req", community, channel, *needed_generation, requester_pseudonym),
        CryptoEvent::MekTransferred { community, channel, generation, sender_pseudonym } =>
            hash_fields!(h, "mek_xfer", community, channel.as_deref().unwrap_or("all"), *generation, sender_pseudonym),
        CryptoEvent::AdminKeypairGranted { community } => hash_fields!(h, "admin_kp", community),
        CryptoEvent::SlotKeypairGranted { community, slot_index, segment_index } =>
            hash_fields!(h, "slot_kp", community, *slot_index, *segment_index),
    }
}

fn hash_voice(h: &mut blake3::Hasher, v: &VoiceEvent) {
    let bucket = now_secs() / 5;
    match v {
        VoiceEvent::Joined { community, channel, pseudonym } => hash_fields!(h, "join", community, channel, pseudonym, bucket),
        VoiceEvent::Left { community, channel, pseudonym } => hash_fields!(h, "leave", community, channel, pseudonym, bucket),
        VoiceEvent::ModeChanged { community, channel, mode, .. } => hash_fields!(h, "mode", community, channel, mode),
        VoiceEvent::MuteChanged { community, channel, target_pseudonym, muted } => hash_fields!(h, "mute", community, channel, target_pseudonym, *muted),
        VoiceEvent::DeafenChanged { community, channel, target_pseudonym, deafened } => hash_fields!(h, "deafen", community, channel, target_pseudonym, *deafened),
        VoiceEvent::RosterUpdated { community, channel, participant_count } => hash_fields!(h, "roster", community, channel, *participant_count),
    }
}

fn hash_governance(h: &mut blake3::Hasher, g: &GovernanceEvent) {
    match g {
        GovernanceEvent::MetadataChanged { community } => hash_fields!(h, "meta", community),
        GovernanceEvent::ChannelsChanged { community } => hash_fields!(h, "channels", community),
        GovernanceEvent::RolesChanged { community } => hash_fields!(h, "roles", community),
        GovernanceEvent::BansChanged { community } => hash_fields!(h, "bans", community),
        GovernanceEvent::InvitesChanged { community } => hash_fields!(h, "invites", community),
        GovernanceEvent::ChannelPermissionsChanged { community, channel } => hash_fields!(h, "perms", community, channel),
        GovernanceEvent::GovernanceSubkeyUpdated { community, subkey_index, lamport_ts } =>
            hash_fields!(h, "subkey", community, *subkey_index, *lamport_ts),
    }
}

fn hash_social(h: &mut blake3::Hasher, s: &SocialEvent) {
    match s {
        SocialEvent::ReactionAdded { community, channel, message_id, emoji, reactor_pseudonym } =>
            hash_fields!(h, "rxn+", community, channel, message_id, emoji, reactor_pseudonym),
        SocialEvent::ReactionRemoved { community, channel, message_id, emoji, reactor_pseudonym } =>
            hash_fields!(h, "rxn-", community, channel, message_id, emoji, reactor_pseudonym),
        SocialEvent::MessagePinned { community, channel, message_id, pinned_by } =>
            hash_fields!(h, "pin+", community, channel, message_id, pinned_by),
        SocialEvent::MessageUnpinned { community, channel, message_id } =>
            hash_fields!(h, "pin-", community, channel, message_id),
        SocialEvent::ThreadCreated { community, thread_id, .. } => hash_fields!(h, "thread+", community, thread_id),
        SocialEvent::ThreadMessagePosted { community, thread_id, message_id, .. } =>
            hash_fields!(h, "thread_msg", community, thread_id, message_id),
        SocialEvent::ThreadArchiveChanged { community, thread_id, archived } =>
            hash_fields!(h, "thread_arc", community, thread_id, *archived),
        SocialEvent::EventCreated { community, event_id, .. } => hash_fields!(h, "event+", community, event_id),
        SocialEvent::EventUpdated { community, event_id, .. } => hash_fields!(h, "event~", community, event_id),
        SocialEvent::EventDeleted { community, event_id } => hash_fields!(h, "event-", community, event_id),
        SocialEvent::EventRsvpChanged { community, event_id, pseudonym, rsvp_status } =>
            hash_fields!(h, "rsvp", community, event_id, pseudonym, rsvp_status),
        SocialEvent::EventReminder { community, event_id, minutes_until_start, .. } =>
            hash_fields!(h, "remind", community, event_id, *minutes_until_start),
        SocialEvent::GameServerAdded { community, server_id, .. } => hash_fields!(h, "game+", community, server_id),
        SocialEvent::GameServerRemoved { community, server_id } => hash_fields!(h, "game-", community, server_id),
    }
}

fn hash_network(h: &mut blake3::Hasher, n: &NetworkEvent) {
    match n {
        NetworkEvent::AttachmentChanged { is_attached, public_internet_ready } =>
            hash_fields!(h, "attach", *is_attached, *public_internet_ready),
        NetworkEvent::LocalRoutesDied { count } => hash_fields!(h, "local_routes", *count),
        NetworkEvent::RemoteRoutesDied { peer_keys } => {
            h.update(b"remote_routes|");
            for k in peer_keys { h.update(k.as_bytes()); h.update(b","); }
        }
        NetworkEvent::WatchRenewed { record_key } => hash_fields!(h, "watch_ok", record_key),
        NetworkEvent::WatchReestablished { record_key } => hash_fields!(h, "watch_re", record_key),
        NetworkEvent::WatchFailed { record_key, error } => hash_fields!(h, "watch_fail", record_key, error),
        NetworkEvent::ValueChanged { record_key, changed_subkeys } => {
            h.update(b"value|"); h.update(record_key.as_bytes());
            for sk in changed_subkeys { h.update(&sk.to_le_bytes()); }
        }
    }
}

fn hash_system(h: &mut blake3::Hasher, s: &SystemEvent) {
    let bucket = now_secs() / 10;
    match s {
        SystemEvent::Announcement { community, body, .. } => {
            h.update(b"announce|");
            h.update(community.as_deref().unwrap_or("global").as_bytes());
            h.update(b"|"); h.update(blake3::hash(body.as_bytes()).as_bytes());
            h.update(b"|"); h.update(&bucket.to_le_bytes());
        }
        SystemEvent::RaidAlert { community, active } => hash_fields!(h, "raid", community, *active),
        SystemEvent::ChannelLockdown { community, locked } => hash_fields!(h, "lockdown", community, *locked),
        SystemEvent::Kicked { community } => hash_fields!(h, "kicked", community),
        SystemEvent::BootstrapRequested { community, joiner_pseudonym } => hash_fields!(h, "boot_req", community, joiner_pseudonym),
        SystemEvent::BootstrapReceived { community } => hash_fields!(h, "boot_recv", community),
        SystemEvent::SyncRequested { community, channel, since_timestamp } =>
            hash_fields!(h, "sync_req", community, channel, *since_timestamp),
        SystemEvent::SyncReceived { community, channel, message_count } =>
            hash_fields!(h, "sync_recv", community, channel, *message_count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::subscription_events::UnreadContext;

    #[test]
    fn same_event_produces_same_hash() {
        let event = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
            from_key: "abc123".into(),
            display_name: "alice".into(),
            message: "hello".into(),
        });
        assert_eq!(hash_event(&event), hash_event(&event));
    }

    #[test]
    fn different_events_produce_different_hashes() {
        let e1 = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
            from_key: "abc123".into(), display_name: "alice".into(), message: "hello".into(),
        });
        let e2 = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
            from_key: "def456".into(), display_name: "bob".into(), message: "hello".into(),
        });
        assert_ne!(hash_event(&e1), hash_event(&e2));
    }

    #[test]
    fn dedup_suppresses_duplicate() {
        let mut dedup = EventDedup::new(100, 300);
        let event = SubscriptionEvent::Friend(FriendEvent::Accepted {
            peer_key: "abc123".into(), dm_log_key: "log1".into(),
        });
        assert!(dedup.check(&event));
        assert!(!dedup.check(&event));
        assert_eq!(dedup.suppressed_count(), 1);
    }

    #[test]
    fn unread_changed_never_deduped() {
        let mut dedup = EventDedup::new(100, 300);
        let event = SubscriptionEvent::UnreadChanged {
            context: UnreadContext::FriendRequests, count: 3,
        };
        assert!(dedup.check(&event));
        assert!(dedup.check(&event));
        assert_eq!(dedup.suppressed_count(), 0);
    }

    #[test]
    fn capacity_eviction() {
        let mut dedup = EventDedup::new(2, 300);
        let e1 = SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: "a".into() });
        let e2 = SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: "b".into() });
        let e3 = SubscriptionEvent::Friend(FriendEvent::Removed { peer_key: "c".into() });
        assert!(dedup.check(&e1));
        assert!(dedup.check(&e2));
        assert!(dedup.check(&e3));
        assert!(dedup.check(&e1)); // e1 evicted, so it's new again
        assert_eq!(dedup.len(), 2);
    }
}
