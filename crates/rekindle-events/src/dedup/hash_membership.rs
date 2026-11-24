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
        MembershipEvent::MembersRefreshed { community } => {
            h.update(b"members_refresh|");
            h.update(community.as_bytes());
        }
        MembershipEvent::MemberDiscovered {
            community,
            pseudonym,
            subkey_index,
            ..
        } => {
            h.update(b"member_disc|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(&subkey_index.to_le_bytes());
        }
        MembershipEvent::JoinProgress {
            community,
            stage,
            status,
        } => {
            // Stage and status both, so `started` and `done` for one
            // phase are two events rather than one deduped away.
            h.update(b"join_prog|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(stage.as_bytes());
            h.update(b"|");
            h.update(status.as_bytes());
        }
        MembershipEvent::SelfJoined { community, .. } => {
            // One join of ours per community per session.
            h.update(b"self_joined|");
            h.update(community.as_bytes());
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
        MembershipEvent::MemberCardChanged {
            community,
            pseudonym,
            display_name,
            custom_status,
        } => {
            // The name and status too: a rename or a new status is a new
            // event, a repeat of the same card one.
            h.update(b"card|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(display_name.as_bytes());
            h.update(b"|");
            h.update(custom_status.as_deref().unwrap_or_default().as_bytes());
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
            ..
        } => {
            h.update(b"kicked|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
        }
        MembershipEvent::Banned {
            community,
            target_pseudonym,
            ..
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
                h.update(id.as_bytes());
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
        // Their row in a new state is its own event (pending, then
        // accepted), never a duplicate of the last one.
        FriendEvent::Added {
            peer_key,
            friendship_state,
            ..
        } => {
            h.update(b"add|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(friendship_state.as_str().as_bytes());
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
        FriendEvent::BioChanged {
            peer_key,
            bio_digest,
        } => {
            h.update(b"bio|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(bio_digest.as_bytes());
        }
        FriendEvent::DisplayNameChanged {
            peer_key,
            display_name,
        } => {
            h.update(b"name|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(display_name.as_bytes());
        }
        FriendEvent::NicknameChanged { peer_key, nickname } => {
            h.update(b"nick|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(nickname.as_deref().unwrap_or_default().as_bytes());
        }
        FriendEvent::GroupChanged {
            peer_key, group, ..
        } => {
            h.update(b"group|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(group.as_deref().unwrap_or_default().as_bytes());
        }
        FriendEvent::KeyChanged { peer_key, new_key } => {
            h.update(b"key|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(new_key.as_bytes());
        }
        FriendEvent::HandshakeFailed { peer_key, reason } => {
            h.update(b"hs_fail|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(reason.as_bytes());
        }
        FriendEvent::HandshakeRetrying { peer_key } => {
            h.update(b"hs_retry|");
            h.update(peer_key.as_bytes());
        }
        FriendEvent::Reachability {
            peer_key,
            reachable,
        } => {
            h.update(b"reach|");
            h.update(peer_key.as_bytes());
            h.update(&[u8::from(*reachable)]);
        }
        FriendEvent::IdentityPinMismatch { peer_key, .. } => {
            h.update(b"pin_mismatch|");
            h.update(peer_key.as_bytes());
        }
        FriendEvent::IdentityPinRestored { peer_key } => {
            h.update(b"pin_restored|");
            h.update(peer_key.as_bytes());
        }
        FriendEvent::VerificationChanged { peer_key, verified } => {
            h.update(b"verified|");
            h.update(peer_key.as_bytes());
            h.update(&[u8::from(*verified)]);
        }
        FriendEvent::GameInvite {
            peer_key,
            game_id,
            server,
        } => {
            h.update(b"game_invite|");
            h.update(peer_key.as_bytes());
            h.update(&game_id.to_le_bytes());
            h.update(server.as_str().as_bytes());
        }
        FriendEvent::CreatorInvited {
            peer_key,
            anchor,
            community_name,
        } => {
            h.update(b"creator_invited|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(anchor.as_bytes());
            h.update(b"|");
            h.update(community_name.as_bytes());
        }
        FriendEvent::CreatorInviteWithdrawn { peer_key, anchor } => {
            h.update(b"creator_withdrawn|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(anchor.as_bytes());
        }
        FriendEvent::CoCreatorStatus {
            peer_key,
            anchor,
            status,
        } => {
            h.update(b"co_creator|");
            h.update(peer_key.as_bytes());
            h.update(b"|");
            h.update(anchor.as_bytes());
            h.update(b"|");
            h.update(status.as_bytes());
        }
        FriendEvent::CoCreationFinished { anchor, created } => {
            h.update(b"co_creation_finished|");
            h.update(anchor.as_bytes());
            h.update(&[u8::from(*created)]);
        }
        FriendEvent::BlockChanged { peer_key, blocked } => {
            h.update(b"blocked|");
            h.update(peer_key.as_bytes());
            h.update(&[u8::from(*blocked)]);
        }
    }
}
