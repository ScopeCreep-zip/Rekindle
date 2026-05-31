//! Cross-tier event deduplication via Blake3 content hashing.
//!
//! Every `SubscriptionEvent` is Blake3-hashed before emission. The hash
//! covers semantically meaningful fields — not pathway-specific metadata
//! like arrival timestamp or source tier. A single bounded `HashSet<[u8; 32]>`
//! rejects duplicates regardless of whether the event arrived via DHT watch,
//! gossip mesh, or periodic poll.
//!
//! At 100K+ agent scale, this ensures the IPC bus carries only unique events.

use std::collections::{HashSet, VecDeque};
use std::time::Instant;

use tracing::{debug, trace};

use rekindle_types::subscription_events::{
    ChannelMessageEvent, CryptoEvent, FriendEvent, GovernanceEvent, MembershipEvent, NetworkEvent,
    PresenceEvent, SocialEvent, SubscriptionEvent, SystemEvent, TypingContext, TypingEvent,
    VoiceEvent,
};

/// Content-addressed event deduplication using Blake3 digests.
///
/// Events are hashed by their canonical content (not pathway metadata).
/// The digest set is bounded by capacity with FIFO eviction. Expired
/// entries are also evicted on periodic cleanup.
pub struct EventDedup {
    /// Set of Blake3 digests for events already emitted.
    digests: HashSet<[u8; 32]>,
    /// FIFO order for eviction when at capacity.
    order: VecDeque<([u8; 32], Instant)>,
    /// Maximum number of digests to retain.
    capacity: usize,
    /// TTL for digest entries — events older than this are evictable.
    ttl_secs: u64,
    /// Count of duplicates suppressed (diagnostic).
    suppressed_count: u64,
}

impl EventDedup {
    /// Create a new dedup cache with the given capacity and TTL.
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
    /// Returns `false` if the event is a duplicate.
    pub fn check(&mut self, event: &SubscriptionEvent) -> bool {
        // UnreadChanged is always emitted — it's a computed aggregate, not a network event
        if matches!(event, SubscriptionEvent::UnreadChanged { .. }) {
            return true;
        }

        let digest = hash_event(event);

        if self.digests.contains(&digest) {
            self.suppressed_count += 1;
            trace!(
                suppressed = self.suppressed_count,
                "dedup: duplicate suppressed"
            );
            return false;
        }

        // Evict oldest if at capacity
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
                break; // ordered by insertion time, so all remaining are newer
            }
        }
        if evicted > 0 {
            debug!(
                evicted,
                remaining = self.digests.len(),
                "dedup: expired entries evicted"
            );
        }
    }

    /// Number of active digest entries.
    pub fn len(&self) -> usize {
        self.digests.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.digests.is_empty()
    }

    /// Total duplicates suppressed since creation.
    pub fn suppressed_count(&self) -> u64 {
        self.suppressed_count
    }
}

impl Default for EventDedup {
    fn default() -> Self {
        Self::new(10_000, 300) // 10K entries, 5-minute TTL
    }
}

// ── Blake3 content hashing ─────────────────────────────────────────────

/// Compute the Blake3 digest of an event's canonical content.
///
/// The hash covers semantically meaningful fields only. Pathway-specific
/// metadata (which tier delivered it, arrival timestamp) is excluded so
/// the same event from different tiers produces the same digest.
fn hash_event(event: &SubscriptionEvent) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();

    // Tag with the outer discriminant to prevent cross-type collisions
    hasher.update(event_discriminant_tag(event).as_bytes());
    hasher.update(b"|");

    match event {
        SubscriptionEvent::ChannelMessage(msg) => hash_channel_message(&mut hasher, msg),
        SubscriptionEvent::Typing(t) => hash_typing(&mut hasher, t),
        SubscriptionEvent::Presence(p) => hash_presence(&mut hasher, p),
        SubscriptionEvent::Membership(m) => hash_membership(&mut hasher, m),
        SubscriptionEvent::Friend(f) => hash_friend(&mut hasher, f),
        SubscriptionEvent::Crypto(c) => hash_crypto(&mut hasher, c),
        SubscriptionEvent::Voice(v) => hash_voice(&mut hasher, v),
        SubscriptionEvent::Governance(g) => hash_governance(&mut hasher, g),
        SubscriptionEvent::Social(s) => hash_social(&mut hasher, s),
        SubscriptionEvent::Network(n) => hash_network(&mut hasher, n),
        SubscriptionEvent::System(s) => hash_system(&mut hasher, s),
        SubscriptionEvent::UnreadChanged { .. } => {
            // Never reaches here — check() returns true early for UnreadChanged
            hasher.update(b"unread");
        }
    }

    *hasher.finalize().as_bytes()
}

fn event_discriminant_tag(event: &SubscriptionEvent) -> &'static str {
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
    }
}

fn hash_channel_message(h: &mut blake3::Hasher, msg: &ChannelMessageEvent) {
    match msg {
        ChannelMessageEvent::New {
            community,
            channel,
            message_id,
            ..
        } => {
            h.update(b"new|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
        }
        ChannelMessageEvent::Edited {
            community,
            channel,
            message_id,
            ..
        } => {
            h.update(b"edited|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
        }
        ChannelMessageEvent::Deleted {
            community,
            channel,
            message_id,
        } => {
            h.update(b"deleted|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
        }
        ChannelMessageEvent::DirectMessageReceived {
            peer_key,
            timestamp,
            ..
        } => {
            h.update(b"dm|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            // Bucketize to 1-second granularity to tolerate clock skew across pathways
            h.update(&(timestamp / 1000).to_le_bytes());
        }
    }
}

fn hash_typing(h: &mut blake3::Hasher, t: &TypingEvent) {
    let now_bucket = rekindle_utils::timestamp_secs() / 5; // 5-second bucketing
    match t {
        TypingEvent::Started { context, who } => {
            h.update(b"start|");
            hash_typing_context(h, context);
            h.update(b"|");
            h.update(who.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        TypingEvent::Stopped { context, who } => {
            h.update(b"stop|");
            hash_typing_context(h, context);
            h.update(b"|");
            h.update(who.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
    }
}

fn hash_typing_context(h: &mut blake3::Hasher, ctx: &TypingContext) {
    match ctx {
        TypingContext::Channel { community, channel } => {
            h.update(b"ch:");
            h.update(community.as_bytes());
            h.update(b":");
            h.update(channel.as_bytes());
        }
        TypingContext::Dm { peer_key } => {
            h.update(b"dm:");
            h.update(peer_key.as_bytes());
        }
    }
}

fn hash_presence(h: &mut blake3::Hasher, p: &PresenceEvent) {
    let now_bucket = rekindle_utils::timestamp_secs() / 30; // 30-second bucketing
    match p {
        PresenceEvent::CommunityMemberChanged {
            community,
            pseudonym,
            status,
            ..
        } => {
            h.update(b"community|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(status.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        PresenceEvent::FriendChanged {
            peer_key, status, ..
        } => {
            h.update(b"friend|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(status.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
    }
}

fn hash_membership(h: &mut blake3::Hasher, m: &MembershipEvent) {
    match m {
        MembershipEvent::JoinRequested {
            community,
            pseudonym,
            ..
        } => {
            h.update(b"join_req|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
        }
        MembershipEvent::JoinAccepted { community, .. } => {
            h.update(b"join_acc|");
            h.update(community.as_bytes());
        }
        MembershipEvent::JoinRejected { community, reason } => {
            h.update(b"join_rej|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(reason.as_bytes());
        }
        MembershipEvent::Joined {
            community,
            pseudonym,
            ..
        } => {
            h.update(b"joined|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
        }
        MembershipEvent::Left {
            community,
            pseudonym,
        } => {
            h.update(b"left|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
        }
        MembershipEvent::Removed {
            community,
            pseudonym,
        } => {
            h.update(b"removed|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
        }
        MembershipEvent::Kicked {
            community,
            target_pseudonym,
        } => {
            h.update(b"kicked|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
        }
        MembershipEvent::Banned {
            community,
            target_pseudonym,
        } => {
            h.update(b"banned|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
        }
        MembershipEvent::Unbanned {
            community,
            target_pseudonym,
        } => {
            h.update(b"unbanned|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
        }
        MembershipEvent::TimedOut {
            community,
            target_pseudonym,
            ..
        } => {
            h.update(b"timeout|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
        }
        MembershipEvent::TimeoutRemoved {
            community,
            target_pseudonym,
        } => {
            h.update(b"timeout_rm|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
        }
        MembershipEvent::TimeoutStatusChanged {
            community,
            pseudonym,
            ..
        } => {
            h.update(b"timeout_st|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
        }
        MembershipEvent::RolesChanged {
            community,
            pseudonym,
            role_ids,
        } => {
            h.update(b"roles|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            for id in role_ids {
                h.update(&id.to_le_bytes());
            }
        }
        MembershipEvent::OnboardingCompleted {
            community,
            pseudonym,
            ..
        } => {
            h.update(b"onboard|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
        }
        MembershipEvent::OnboardingAnswersSubmitted {
            community,
            sender_pseudonym,
            ..
        } => {
            h.update(b"onboard_ans|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(sender_pseudonym.as_bytes());
        }
    }
}

fn hash_friend(h: &mut blake3::Hasher, f: &FriendEvent) {
    match f {
        FriendEvent::RequestReceived { from_key, .. } => {
            h.update(b"req|");
            h.update(from_key.as_bytes());
        }
        FriendEvent::RequestAcknowledged { peer_key } => {
            h.update(b"ack|");
            h.update(peer_key.as_bytes());
        }
        FriendEvent::Accepted { peer_key, .. } => {
            h.update(b"acc|");
            h.update(peer_key.as_bytes());
        }
        FriendEvent::Rejected { peer_key } => {
            h.update(b"rej|");
            h.update(peer_key.as_bytes());
        }
        FriendEvent::Removed { peer_key } => {
            h.update(b"rem|");
            h.update(peer_key.as_bytes());
        }
        FriendEvent::RemoveAcknowledged { peer_key } => {
            h.update(b"rem_ack|");
            h.update(peer_key.as_bytes());
        }
        FriendEvent::ProfileKeyRotated {
            peer_key,
            new_profile_dht_key,
        } => {
            h.update(b"rot|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(new_profile_dht_key.as_bytes());
        }
    }
}

fn hash_crypto(h: &mut blake3::Hasher, c: &CryptoEvent) {
    match c {
        CryptoEvent::MekRotated {
            community,
            channel,
            generation,
            ..
        } => {
            h.update(b"mek_rot|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_deref().unwrap_or("all").as_bytes());
            h.update(b"|");
            h.update(&generation.to_le_bytes());
        }
        CryptoEvent::MekRequested {
            community,
            channel,
            needed_generation,
            requester_pseudonym,
        } => {
            h.update(b"mek_req|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(&needed_generation.to_le_bytes());
            h.update(b"|");
            h.update(requester_pseudonym.as_bytes());
        }
        CryptoEvent::MekTransferred {
            community,
            channel,
            generation,
            sender_pseudonym,
        } => {
            h.update(b"mek_xfer|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_deref().unwrap_or("all").as_bytes());
            h.update(b"|");
            h.update(&generation.to_le_bytes());
            h.update(b"|");
            h.update(sender_pseudonym.as_bytes());
        }
        CryptoEvent::AdminKeypairGranted { community } => {
            h.update(b"admin_kp|");
            h.update(community.as_bytes());
        }
        CryptoEvent::SlotKeypairGranted {
            community,
            slot_index,
            segment_index,
        } => {
            h.update(b"slot_kp|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(&slot_index.to_le_bytes());
            h.update(b"|");
            h.update(&segment_index.to_le_bytes());
        }
        CryptoEvent::PqBundlePublished { subkey, kind } => {
            h.update(b"pq_bundle|");
            h.update(&subkey.to_le_bytes());
            h.update(b"|");
            h.update(match kind {
                rekindle_types::subscription_events::PqBundleKind::LastResort => b"lr".as_slice(),
                rekindle_types::subscription_events::PqBundleKind::OneTimeBatch => b"ot".as_slice(),
            });
        }
    }
}

fn hash_voice(h: &mut blake3::Hasher, v: &VoiceEvent) {
    let now_bucket = rekindle_utils::timestamp_secs() / 5;
    match v {
        VoiceEvent::Joined {
            community,
            channel,
            pseudonym,
        } => {
            h.update(b"join|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::Left {
            community,
            channel,
            pseudonym,
        } => {
            h.update(b"leave|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::ModeChanged {
            community,
            channel,
            mode,
            ..
        } => {
            h.update(b"mode|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(mode.as_bytes());
        }
        VoiceEvent::MuteChanged {
            community,
            channel,
            target_pseudonym,
            muted,
        } => {
            h.update(b"mute|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*muted)]);
        }
        VoiceEvent::DeafenChanged {
            community,
            channel,
            target_pseudonym,
            deafened,
        } => {
            h.update(b"deafen|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*deafened)]);
        }
        VoiceEvent::RosterUpdated {
            community,
            channel,
            participant_count,
        } => {
            h.update(b"roster|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(&(*participant_count as u64).to_le_bytes());
        }
    }
}

fn hash_governance(h: &mut blake3::Hasher, g: &GovernanceEvent) {
    match g {
        GovernanceEvent::MetadataChanged { community } => {
            h.update(b"meta|");
            h.update(community.as_bytes());
        }
        GovernanceEvent::ChannelsChanged { community } => {
            h.update(b"channels|");
            h.update(community.as_bytes());
        }
        GovernanceEvent::RolesChanged { community } => {
            h.update(b"roles|");
            h.update(community.as_bytes());
        }
        GovernanceEvent::BansChanged { community } => {
            h.update(b"bans|");
            h.update(community.as_bytes());
        }
        GovernanceEvent::InvitesChanged { community } => {
            h.update(b"invites|");
            h.update(community.as_bytes());
        }
        GovernanceEvent::ChannelPermissionsChanged { community, channel } => {
            h.update(b"perms|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
        }
        GovernanceEvent::GovernanceSubkeyUpdated {
            community,
            subkey_index,
            lamport_ts,
        } => {
            h.update(b"subkey|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(&subkey_index.to_le_bytes());
            h.update(b"|");
            h.update(&lamport_ts.to_le_bytes());
        }
    }
}

fn hash_social(h: &mut blake3::Hasher, s: &SocialEvent) {
    match s {
        SocialEvent::ReactionAdded {
            community,
            channel,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            h.update(b"rxn+|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
            h.update(b"|");
            h.update(emoji.as_bytes());
            h.update(b"|");
            h.update(reactor_pseudonym.as_bytes());
        }
        SocialEvent::ReactionRemoved {
            community,
            channel,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            h.update(b"rxn-|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
            h.update(b"|");
            h.update(emoji.as_bytes());
            h.update(b"|");
            h.update(reactor_pseudonym.as_bytes());
        }
        SocialEvent::MessagePinned {
            community,
            channel,
            message_id,
            pinned_by,
        } => {
            h.update(b"pin+|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
            h.update(b"|");
            h.update(pinned_by.as_bytes());
        }
        SocialEvent::MessageUnpinned {
            community,
            channel,
            message_id,
        } => {
            h.update(b"pin-|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
        }
        SocialEvent::ThreadCreated {
            community,
            thread_id,
            ..
        } => {
            h.update(b"thread+|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(thread_id.as_bytes());
        }
        SocialEvent::ThreadMessagePosted {
            community,
            thread_id,
            message_id,
            ..
        } => {
            h.update(b"thread_msg|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(thread_id.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
        }
        SocialEvent::ThreadArchiveChanged {
            community,
            thread_id,
            archived,
        } => {
            h.update(b"thread_arc|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(thread_id.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*archived)]);
        }
        SocialEvent::EventCreated {
            community,
            event_id,
            ..
        } => {
            h.update(b"event+|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(event_id.as_bytes());
        }
        SocialEvent::EventUpdated {
            community,
            event_id,
            ..
        } => {
            h.update(b"event~|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(event_id.as_bytes());
        }
        SocialEvent::EventDeleted {
            community,
            event_id,
        } => {
            h.update(b"event-|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(event_id.as_bytes());
        }
        SocialEvent::EventRsvpChanged {
            community,
            event_id,
            pseudonym,
            rsvp_status,
        } => {
            h.update(b"rsvp|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(event_id.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(rsvp_status.as_bytes());
        }
        SocialEvent::EventReminder {
            community,
            event_id,
            minutes_until_start,
            ..
        } => {
            h.update(b"remind|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(event_id.as_bytes());
            h.update(b"|");
            h.update(&minutes_until_start.to_le_bytes());
        }
        SocialEvent::GameServerAdded {
            community,
            server_id,
            ..
        } => {
            h.update(b"game+|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(server_id.as_bytes());
        }
        SocialEvent::GameServerRemoved {
            community,
            server_id,
        } => {
            h.update(b"game-|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(server_id.as_bytes());
        }
    }
}

fn hash_network(h: &mut blake3::Hasher, n: &NetworkEvent) {
    match n {
        NetworkEvent::AttachmentChanged {
            is_attached,
            public_internet_ready,
        } => {
            h.update(b"attach|");
            h.update(&[u8::from(*is_attached), u8::from(*public_internet_ready)]);
        }
        NetworkEvent::LocalRoutesDied { count } => {
            h.update(b"local_routes|");
            h.update(&(*count as u64).to_le_bytes());
        }
        NetworkEvent::RemoteRoutesDied { peer_keys } => {
            h.update(b"remote_routes|");
            for k in peer_keys {
                h.update(k.as_bytes());
                h.update(b",");
            }
        }
        NetworkEvent::WatchRenewed { record_key } => {
            h.update(b"watch_ok|");
            h.update(record_key.as_bytes());
        }
        NetworkEvent::WatchReestablished { record_key } => {
            h.update(b"watch_re|");
            h.update(record_key.as_bytes());
        }
        NetworkEvent::WatchFailed { record_key, error } => {
            h.update(b"watch_fail|");
            h.update(record_key.as_bytes());
            h.update(b"|");
            h.update(error.as_bytes());
        }
        NetworkEvent::ValueChanged {
            record_key,
            changed_subkeys,
        } => {
            h.update(b"value|");
            h.update(record_key.as_bytes());
            for sk in changed_subkeys {
                h.update(&sk.to_le_bytes());
            }
        }
    }
}

fn hash_system(h: &mut blake3::Hasher, s: &SystemEvent) {
    let now_bucket = rekindle_utils::timestamp_secs() / 10; // 10-second bucketing
    match s {
        SystemEvent::Announcement {
            community, body, ..
        } => {
            h.update(b"announce|");
            h.update(community.as_deref().unwrap_or("global").as_bytes());
            h.update(b"|");
            // Hash the body content, not the full body (avoid length-based collisions)
            let body_hash = blake3::hash(body.as_bytes());
            h.update(body_hash.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        SystemEvent::RaidAlert { community, active } => {
            h.update(b"raid|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*active)]);
        }
        SystemEvent::ChannelLockdown { community, locked } => {
            h.update(b"lockdown|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*locked)]);
        }
        SystemEvent::Kicked { community } => {
            h.update(b"kicked|");
            h.update(community.as_bytes());
        }
        SystemEvent::BootstrapRequested {
            community,
            joiner_pseudonym,
        } => {
            h.update(b"boot_req|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(joiner_pseudonym.as_bytes());
        }
        SystemEvent::BootstrapReceived { community } => {
            h.update(b"boot_recv|");
            h.update(community.as_bytes());
        }
        SystemEvent::SyncRequested {
            community,
            channel,
            since_timestamp,
        } => {
            h.update(b"sync_req|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(&since_timestamp.to_le_bytes());
        }
        SystemEvent::SyncReceived {
            community,
            channel,
            message_count,
        } => {
            h.update(b"sync_recv|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(&(*message_count as u64).to_le_bytes());
        }
        SystemEvent::AuditChainBroken { cursor } => {
            h.update(b"audit_broken|");
            h.update(&cursor.to_le_bytes());
        }
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
        let h1 = hash_event(&event);
        let h2 = hash_event(&event);
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_events_produce_different_hashes() {
        let e1 = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
            from_key: "abc123".into(),
            display_name: "alice".into(),
            message: "hello".into(),
        });
        let e2 = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
            from_key: "def456".into(),
            display_name: "bob".into(),
            message: "hello".into(),
        });
        assert_ne!(hash_event(&e1), hash_event(&e2));
    }

    #[test]
    fn dedup_suppresses_duplicate() {
        let mut dedup = EventDedup::new(100, 300);
        let event = SubscriptionEvent::Friend(FriendEvent::Accepted {
            peer_key: "abc123".into(),
            dm_log_key: "log1".into(),
        });
        assert!(dedup.check(&event)); // first: emit
        assert!(!dedup.check(&event)); // second: suppress
        assert_eq!(dedup.suppressed_count(), 1);
    }

    #[test]
    fn unread_changed_never_deduped() {
        let mut dedup = EventDedup::new(100, 300);
        let event = SubscriptionEvent::UnreadChanged {
            context: UnreadContext::FriendRequests,
            count: 3,
        };
        assert!(dedup.check(&event));
        assert!(dedup.check(&event)); // same event, still emitted
        assert_eq!(dedup.suppressed_count(), 0);
    }

    #[test]
    fn capacity_eviction() {
        let mut dedup = EventDedup::new(2, 300);
        let e1 = SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: "a".into(),
        });
        let e2 = SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: "b".into(),
        });
        let e3 = SubscriptionEvent::Friend(FriendEvent::Removed {
            peer_key: "c".into(),
        });

        assert!(dedup.check(&e1));
        assert!(dedup.check(&e2));
        assert!(dedup.check(&e3)); // evicts e1
        assert!(dedup.check(&e1)); // e1 was evicted, so it's new again
        assert_eq!(dedup.len(), 2);
    }
}
