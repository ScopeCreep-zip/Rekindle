//! Content hashers for governance-state-change and social (reactions,
//! pins, threads, events, game servers) subscription events.

use rekindle_types::subscription_events::{GovernanceEvent, SocialEvent};

pub(super) fn hash_governance(h: &mut blake3::Hasher, g: &GovernanceEvent) {
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

pub(super) fn hash_social(h: &mut blake3::Hasher, s: &SocialEvent) {
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
