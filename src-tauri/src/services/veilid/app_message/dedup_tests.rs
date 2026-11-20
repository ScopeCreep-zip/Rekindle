//! Linux walk row M25: a `VoiceLeave` 0.2 s after a rejoin never reached
//! the peer. The shell keyed its gossip dedup by a content hash over a
//! `Control` envelope, and every leave of one channel is byte-identical,
//! so the peer dropped the second leave (both its direct copy and its
//! gossip copy) as the first leave's duplicate and kept the member.

use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, SignedEnvelope};
use rekindle_codec::dedup::DedupCache;

const WALK_COMMUNITY: &str = "community";
const WALK: &str = "walk";

/// A `VoiceLeave` as the walk sends it, with the send's `message_id`.
fn leave(message_id: u8) -> SignedEnvelope {
    let envelope = CommunityEnvelope::Control(ControlPayload::VoiceLeave {
        channel_id: "voice".into(),
    });
    SignedEnvelope {
        community_id: WALK_COMMUNITY.into(),
        sender_pseudonym: WALK.into(),
        message_id: [message_id; rekindle_codec::envelope::MESSAGE_ID_LEN],
        envelope_bytes: encode_community_envelope(&envelope).expect("encodes"),
        signature: Vec::new(),
        ttl: 0,
    }
}

fn admits(peer: &mut DedupCache, signed: &SignedEnvelope) -> bool {
    !peer.check_and_insert(WALK_COMMUNITY, WALK, &super::gossip_dedup_key(signed))
}

#[test]
fn a_leave_after_a_rejoin_is_not_the_first_leaves_duplicate() {
    let mut peer = DedupCache::new(1024);
    let first = leave(1);
    assert!(admits(&mut peer, &first), "the first leave is applied");
    // The same send arriving again (its gossip copy beside the direct one).
    assert!(!admits(&mut peer, &first), "one send is applied once");

    let after_rejoin = leave(2);
    assert_eq!(first.envelope_bytes, after_rejoin.envelope_bytes);
    assert!(
        admits(&mut peer, &after_rejoin),
        "the leave after a rejoin must reach the voice leave handler"
    );
}
