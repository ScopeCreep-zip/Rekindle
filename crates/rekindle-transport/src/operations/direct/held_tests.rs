use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};
use rekindle_types::friend::FriendshipState;

use super::*;

const NOW: u64 = 1_000_000_000;

fn opened(peer: &str, seq: u64) -> Opened {
    Opened {
        sender_hex: peer.to_string(),
        type_id: DmTypeId::DmText,
        sent_at_ms: NOW,
        body: DirectBody::SessionResetAccept,
        unprocessed: None,
        nonce: None,
        stream_seq: seq,
        chain_seq: seq,
        held: true,
    }
}

#[test]
fn only_user_content_from_a_peer_we_asked_is_held() {
    let content = [
        DmTypeId::DmText,
        DmTypeId::DmEdit,
        DmTypeId::DmDelete,
        DmTypeId::DmReaction,
    ];
    for kind in content {
        assert!(holds(kind, Some(FriendshipState::PendingOut)), "{kind:?}");
        for status in [
            None,
            Some(FriendshipState::Accepted),
            Some(FriendshipState::Removing),
        ] {
            assert!(!holds(kind, status), "{kind:?} {status:?}");
        }
    }
    for kind in [
        DmTypeId::FriendAccept,
        DmTypeId::DmAck,
        DmTypeId::Typing,
        DmTypeId::SessionResetRequest,
    ] {
        assert!(!holds(kind, Some(FriendshipState::PendingOut)), "{kind:?}");
    }
}

#[test]
fn a_peer_holds_at_most_its_cap_and_others_keep_theirs() {
    let mut held = HeldMessages::default();
    for seq in 0..MAX_HELD_PER_PEER {
        let seq = u64::try_from(seq).expect("fits");
        assert!(held.hold(opened("ida", seq), NOW).is_ok());
    }
    let refused = held
        .hold(opened("ida", 9_999), NOW)
        .expect_err("past the cap");
    assert_eq!(refused.stream_seq, 9_999, "the newest is handed back");
    assert_eq!(held.len("ida"), MAX_HELD_PER_PEER);
    assert!(held.hold(opened("jon", 1), NOW).is_ok());
}

#[test]
fn take_returns_the_live_in_order_and_the_expired_apart() {
    let mut held = HeldMessages::default();
    held.hold(opened("ida", 1), NOW).expect("held");
    held.hold(opened("ida", 2), NOW + HELD_EXPIRY_MS)
        .expect("held");
    held.hold(opened("ida", 3), NOW + HELD_EXPIRY_MS + 1)
        .expect("held");
    let (live, expired) = held.take("ida", NOW + HELD_EXPIRY_MS + 1);
    let seqs = |v: &[Opened]| v.iter().map(|o| o.stream_seq).collect::<Vec<_>>();
    assert_eq!(seqs(&live), [2, 3]);
    assert_eq!(seqs(&expired), [1]);
    assert_eq!(held.len("ida"), 0, "taken");
    let (live, expired) = held.take("ida", NOW);
    assert!(live.is_empty() && expired.is_empty());
}

#[test]
fn the_bounds_are_one_ack_and_the_outbox_deadline() {
    assert_eq!(MAX_HELD_PER_PEER, MAX_DM_ACK_SEQS);
    assert_eq!(
        u128::from(HELD_EXPIRY_MS),
        rekindle_crypto::horizon::DELIVERY_HORIZON.as_millis()
    );
}
