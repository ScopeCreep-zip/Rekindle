//! Content hashers for community membership and friend events.

use rekindle_types::subscription_events::{FriendEvent, MembershipEvent};

pub(super) fn hash_membership(h: &mut blake3::Hasher, m: &MembershipEvent) {
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

pub(super) fn hash_friend(h: &mut blake3::Hasher, f: &FriendEvent) {
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
