use rekindle_codec::capnp_codec::direct::{DmReaction, DmText, Typing};

use super::*;

/// A diff's ack carries `through` alone (finding 76b), and joins the
/// seqs already held for the same flush.
#[test]
fn a_moved_cursor_owes_a_through_ack() {
    let mut acks = PendingAcks::default();
    assert!(acks.add_through("p"));
    let held = acks.take("p", false).expect("an ack is owed");
    assert_eq!(held.seqs, Some(BTreeSet::new()));
    assert_eq!(super::acks(&[], 3).len(), 1);
    assert!(acks.add("p", 5));
    assert!(!acks.add_through("p"));
    assert_eq!(
        acks.take("p", false).unwrap().seqs,
        Some(BTreeSet::from([5]))
    );
}

#[test]
fn the_delay_stays_within_two_to_ten_seconds() {
    assert_eq!(delay_from(0), Duration::from_secs(2));
    assert_eq!(delay_from(u64::MAX), Duration::from_secs(10));
    let rng = rekindle_utils::OsRandom;
    for _ in 0..1000 {
        let d = delay(&rng);
        assert!((Duration::from_secs(2)..=Duration::from_secs(10)).contains(&d));
    }
}

#[test]
fn acks_coalesce_per_peer_and_stay_held_while_invisible() {
    let mut acks = PendingAcks::default();
    assert!(acks.add("bob", 3));
    assert!(!acks.add("bob", 1), "one flush per peer");
    assert!(acks.add("carol", 1));
    assert_eq!(acks.take("bob", true), None);
    assert_eq!(acks.held_peers().len(), 2);
    assert!(acks.add("bob", 4), "a held peer schedules again");
    let seqs: Vec<u64> = acks
        .take("bob", false)
        .unwrap()
        .seqs
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(seqs, [1, 3, 4]);
}

#[test]
fn the_read_marker_shares_the_flush_and_is_held_while_invisible() {
    let mut acks = PendingAcks::default();
    assert!(acks.add_read("bob", 4));
    assert!(!acks.add_read("bob", 2), "one flush per peer");
    assert!(!acks.add("bob", 7), "acks ride the same flush");
    // Invisible: nothing leaves, and the marker stays held.
    assert_eq!(acks.take("bob", true), None);
    assert_eq!(acks.held_peers(), ["bob"]);
    assert!(acks.add_read("bob", 6), "a held peer schedules again");
    // Leaving Invisible sends one marker, the highest.
    let held = acks.take("bob", false).unwrap();
    assert_eq!(held.read_up_to, Some(6));
    assert_eq!(held.seqs.unwrap().into_iter().collect::<Vec<_>>(), [7]);
    assert_eq!(acks.take("bob", false), None, "sent once");
    assert!(acks.held_peers().is_empty());
}

#[test]
fn only_an_exact_copy_of_an_opened_entry_is_acknowledged_by_its_seq() {
    let mut acks = PendingAcks::default();
    let (frame, forged) = (super::digest(b"frame 4"), super::digest(b"frame 4'"));
    acks.remember("bob", frame, 4);
    assert_eq!(acks.applied_seq("bob", &frame), Some(4));
    assert_eq!(acks.applied_seq("bob", &forged), None, "another ciphertext");
    assert_eq!(acks.applied_seq("eve", &frame), None, "another sender");
    for seq in 5..5 + APPLIED_KEPT as u64 {
        acks.remember("bob", super::digest(&seq.to_be_bytes()), seq);
    }
    assert_eq!(acks.applied_seq("bob", &frame), None, "bounded");
    acks.clear();
    assert_eq!(
        acks.applied_seq("bob", &super::digest(&5u64.to_be_bytes())),
        None
    );
}

#[test]
fn only_text_is_acknowledged() {
    let text = DirectBody::DmText(DmText {
        client_id: [1; 16],
        body: "hi".into(),
        reply_to: None,
        expire_seconds: 0,
    });
    assert!(acknowledged_id(&text).is_some());
    assert_eq!(
        acknowledged_id(&DirectBody::Typing(Typing { typing: true })),
        None
    );
    assert_eq!(acknowledged_id(&DirectBody::Unfriended), None);
    let reaction = DirectBody::DmReaction(DmReaction {
        target_client_id: [9; 16],
        emoji: "x".into(),
        add: true,
    });
    assert_eq!(acknowledged_id(&reaction), None);
}
