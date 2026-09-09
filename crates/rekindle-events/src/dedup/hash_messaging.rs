//! Content hashers for channel messages, typing indicators, and presence.

use rekindle_types::subscription_events::{
    ChannelMessageEvent, PresenceEvent, TypingContext, TypingEvent,
};

pub(super) fn hash_channel_message(h: &mut blake3::Hasher, msg: &ChannelMessageEvent) {
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

pub(super) fn hash_typing(h: &mut blake3::Hasher, t: &TypingEvent) {
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

pub(super) fn hash_presence(h: &mut blake3::Hasher, p: &PresenceEvent) {
    let now_bucket = rekindle_utils::timestamp_secs() / 30; // 30-second bucketing
                                                            // Every variant hashes subject + status + game identity + bucket.
                                                            // `elapsed_seconds` is deliberately excluded: it advances on every
                                                            // scan, so folding it in would give each tick a distinct hash and
                                                            // dedup would never fire. Game *identity* is what makes a new
                                                            // event; the elapsed count rides along on whichever emission wins.
    let snapshot = p.snapshot();
    match p {
        PresenceEvent::CommunityMemberChanged {
            community,
            pseudonym,
            ..
        } => {
            h.update(b"community|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
        }
        PresenceEvent::SelfChanged { public_key, .. } => {
            h.update(b"self|");
            h.update(public_key.as_bytes());
        }
        PresenceEvent::FriendChanged { peer_key, .. } => {
            h.update(b"friend|");
            h.update(peer_key.as_bytes());
        }
    }
    h.update(b"|");
    h.update(snapshot.status.as_deref().unwrap_or("").as_bytes());
    h.update(b"|");
    h.update(snapshot.game_name().unwrap_or("").as_bytes());
    h.update(b"|");
    h.update(&snapshot.game_id().unwrap_or(0).to_le_bytes());
    h.update(b"|");
    h.update(&now_bucket.to_le_bytes());
}
