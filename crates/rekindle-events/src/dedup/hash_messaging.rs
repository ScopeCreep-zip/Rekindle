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
        ChannelMessageEvent::ChannelSendStatus {
            community,
            channel,
            message_id,
            state,
        } => {
            h.update(b"channel_send_status|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
            h.update(b"|");
            h.update(state.as_str().as_bytes());
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
        ChannelMessageEvent::DmSendStatus {
            client_id,
            conversation_id,
            state,
        } => {
            h.update(b"dm_state|");
            h.update(conversation_id.as_bytes());
            h.update(b"|");
            h.update(client_id.as_bytes());
            h.update(b"|");
            h.update(state.as_str().as_bytes());
        }
        ChannelMessageEvent::DmHistoryGap {
            peer_key,
            through_seq,
        } => {
            h.update(b"dm_gap|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(&through_seq.to_le_bytes());
        }
        ChannelMessageEvent::ConversationFocusRequested {
            peer_key, reason, ..
        } => {
            h.update(b"focus|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(reason.as_bytes());
        }
    }
}

pub(super) fn hash_typing(h: &mut blake3::Hasher, t: &TypingEvent, now_ms: u64) {
    let now_bucket = now_ms / 1000 / 5; // 5-second bucketing
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
        TypingContext::GroupDm { group_id } => {
            h.update(b"gdm:");
            h.update(group_id.as_bytes());
        }
    }
}

pub(super) fn hash_presence(h: &mut blake3::Hasher, p: &PresenceEvent, now_ms: u64) {
    let now_bucket = now_ms / 1000 / 30; // 30-second bucketing
                                         // Every variant hashes subject + status + game identity + bucket.
                                         // `elapsed_seconds` is deliberately excluded: it advances on every
                                         // scan, so folding it in would give each tick a distinct hash and
                                         // dedup would never fire. Game *identity* is what makes a new
                                         // event; the elapsed count rides along on whichever emission wins.
    let snapshot = match p {
        // Our own profile change is published once, by the host, and never
        // arrives over a peer path, so nothing repeats it to dedup.
        PresenceEvent::SelfProfileChanged {
            public_key,
            display_name,
            avatar_changed,
        } => {
            h.update(b"self-profile|");
            h.update(public_key.as_bytes());
            h.update(b"|");
            h.update(display_name.as_deref().unwrap_or("").as_bytes());
            h.update(&[u8::from(*avatar_changed)]);
            h.update(&now_bucket.to_le_bytes());
            return;
        }
        PresenceEvent::CommunityMemberChanged { snapshot, .. }
        | PresenceEvent::FriendChanged { snapshot, .. }
        | PresenceEvent::SelfChanged { snapshot, .. } => snapshot,
    };
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
        PresenceEvent::SelfProfileChanged { .. } => {}
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

/// A group-DM event: its group, its kind and what tells two apart (the
/// member, the epoch, the send and its state).
pub(super) fn hash_group_dm(
    h: &mut blake3::Hasher,
    e: &rekindle_types::subscription_events::GroupDmEvent,
) {
    use rekindle_types::subscription_events::GroupDmEvent;
    h.update(e.group_id().as_bytes());
    h.update(b"|");
    match e {
        GroupDmEvent::Invited { inviter, .. } => {
            h.update(b"inv|");
            h.update(inviter.public_key.as_bytes());
        }
        GroupDmEvent::Joined { member, .. } => {
            h.update(b"join|");
            h.update(member.public_key.as_bytes());
        }
        GroupDmEvent::Left { member, .. } => {
            h.update(b"left|");
            h.update(member.public_key.as_bytes());
        }
        GroupDmEvent::KeyRotated { member, epoch, .. } => {
            h.update(b"rot|");
            h.update(member.public_key.as_bytes());
            h.update(&epoch.to_be_bytes());
        }
        GroupDmEvent::SendStatus {
            client_id, state, ..
        } => {
            h.update(b"send|");
            h.update(client_id.as_bytes());
            h.update(state.as_str().as_bytes());
        }
        GroupDmEvent::Message {
            sender, message_id, ..
        } => {
            h.update(b"msg|");
            h.update(sender.public_key.as_bytes());
            h.update(message_id.as_bytes());
        }
        GroupDmEvent::HistoryGap {
            member,
            through_seq,
            ..
        } => {
            h.update(b"gap|");
            h.update(member.public_key.as_bytes());
            h.update(b"|");
            h.update(&through_seq.to_le_bytes());
        }
    }
}
