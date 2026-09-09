//! Content hashers for governance-state-change and social (reactions,
//! pins, threads, events, game servers) subscription events.

use rekindle_types::subscription_events::{GovernanceEvent, SocialEvent};

pub(super) fn hash_governance(h: &mut blake3::Hasher, g: &GovernanceEvent) {
    // Community first, since every variant has one.
    h.update(g.community().as_bytes());
    h.update(b"|");
    match g {
        // The payload variants hash their *contents*, not just the
        // community: two role-table changes in a row are two events,
        // and hashing the community alone would dedup the second away
        // and leave the UI on the stale table.
        GovernanceEvent::MetadataChanged {
            name,
            description,
            icon_hash,
            banner_hash,
            ..
        } => {
            h.update(b"meta|");
            for field in [name, description, icon_hash, banner_hash] {
                h.update(field.as_deref().unwrap_or("\u{0}").as_bytes());
                h.update(b"|");
            }
        }
        GovernanceEvent::ChannelsChanged {
            channels,
            categories,
            ..
        } => {
            h.update(b"channels|");
            for c in channels {
                h.update(c.id.as_bytes());
                h.update(c.name.as_bytes());
                h.update(&c.sort_order.to_le_bytes());
                h.update(&c.slowmode_seconds.unwrap_or(u32::MAX).to_le_bytes());
                h.update(b",");
            }
            h.update(b"|");
            for c in categories {
                h.update(c.id.as_bytes());
                h.update(c.name.as_bytes());
                h.update(&c.sort_order.to_le_bytes());
                h.update(b",");
            }
        }
        GovernanceEvent::RolesChanged { roles, .. } => {
            h.update(b"roles|");
            for r in roles {
                h.update(&r.id.to_le_bytes());
                h.update(r.name.as_bytes());
                h.update(&r.permissions.to_le_bytes());
                h.update(&r.position.to_le_bytes());
                h.update(&[
                    u8::from(r.hoist),
                    u8::from(r.mentionable),
                    u8::from(r.self_assignable),
                ]);
                h.update(r.exclusion_group.as_deref().unwrap_or("").as_bytes());
                h.update(b",");
            }
        }
        GovernanceEvent::BansChanged { .. } => {
            h.update(b"bans");
        }
        GovernanceEvent::InviteCreated { code_hash, .. } => {
            h.update(b"invite_new|");
            h.update(code_hash.as_bytes());
        }
        GovernanceEvent::InviteUsed {
            code_hash, uses, ..
        } => {
            // The use count is part of the identity: consecutive
            // redemptions of one invite are distinct events.
            h.update(b"invite_used|");
            h.update(code_hash.as_bytes());
            h.update(b"|");
            h.update(&uses.to_le_bytes());
        }
        GovernanceEvent::InviteRevoked { code_hash, .. } => {
            h.update(b"invite_rev|");
            h.update(code_hash.as_bytes());
        }
        GovernanceEvent::GovernanceRebuilt { .. } => {
            h.update(b"rebuilt");
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
