use std::time::{Duration, Instant};

use super::*;

fn secret(b: u8) -> Zeroizing<[u8; 32]> {
    Zeroizing::new([b; 32])
}

fn rotated(r: LeaveRotation) -> OwnKey {
    match r {
        LeaveRotation::Rotated(k) => k,
        LeaveRotation::Deferred(_) => panic!("expected a rotation"),
    }
}

#[test]
fn a_join_ratchets_a_used_key_and_the_joiner_gets_only_the_new_step() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new();
    let before = keys.send_key(now);
    let join = keys.on_join("bob", now);
    assert!(join.everyone.is_none(), "a ratchet goes to the joiner only");
    assert_eq!(join.joiner.len(), 1);
    let after = &join.joiner[0];
    assert_eq!(after.index, before.index + 1);
    assert_eq!(*after.secret, *sframe::ratchet(&before.secret));
    assert_eq!(keys.send_key(now).index, after.index, "used at once");
}

#[test]
fn joins_while_the_key_is_unused_coalesce_into_one_step() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new();
    let first = keys.send_key(now).index;
    let a = keys.on_join("bob", now);
    let b = keys.on_join("carol", now);
    assert_eq!(a.joiner[0].index, first + 1);
    assert_eq!(b.joiner[0].index, first + 1);
}

#[test]
fn a_join_at_the_last_step_starts_a_generation_the_joiner_waits_for() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new();
    for i in 0..LAST_STEP {
        keys.send_key(now);
        keys.on_join(&format!("p{i}"), now);
    }
    let last = keys.send_key(now);
    assert_eq!(step(last.index), LAST_STEP);
    let join = keys.on_join("late", now);
    let everyone = join.everyone.expect("a new generation for everyone");
    assert_eq!(generation(everyone.index), generation(last.index) + 1);
    assert_eq!(join.joiner.len(), 1);
    assert_eq!(join.joiner[0].index, everyone.index, "never the old step");
    assert_eq!(keys.shareable_for("late")[0].index, everyone.index);
}

#[test]
fn a_leave_starts_a_new_generation_used_after_its_delay() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new();
    let first = keys.send_key(now);
    let next = rotated(keys.on_leave(now));
    assert_eq!(generation(next.index), generation(first.index) + 1);
    assert_eq!(step(next.index), 0);
    assert_eq!(
        keys.send_key(now).index,
        first.index,
        "not before the delay"
    );
    assert_eq!(keys.send_key(now + USE_DELAY).index, next.index);
}

#[test]
fn a_leave_during_the_delay_defers_a_second_rotation_and_never_rebinds_an_index() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new();
    let a = rotated(keys.on_leave(now));
    let LeaveRotation::Deferred(at) = keys.on_leave(now + Duration::from_secs(1)) else {
        panic!("one rotation pending at a time");
    };
    assert_eq!(at, now + USE_DELAY);
    let b = keys.follow_up(at).expect("the deferred rotation starts");
    assert_eq!(generation(b.index), generation(a.index) + 1);
    assert_eq!(keys.send_key(at).index, a.index, "the first one applied");
    assert!(keys.follow_up(at).is_none(), "only once");
}

fn note(k: &OwnKey, bound: &mut HashMap<u64, [u8; 32]>) {
    let prior = bound.insert(k.index, *k.secret);
    assert!(
        prior.is_none_or(|s| s == *k.secret),
        "index {} re-bound",
        k.index
    );
}

/// A leave every 2 s for 60 s: every index is bound to one secret, and a
/// leaver's last key is out of use within 10 s.
#[test]
fn rapid_leaves_bind_each_index_once_and_retire_the_leavers_key_in_time() {
    let start = Instant::now();
    let keys = ChannelSenderKeys::new();
    let mut bound: HashMap<u64, [u8; 32]> = HashMap::new();
    let mut follow_ups: Vec<Instant> = Vec::new();
    for tick in 0..=60u64 {
        let now = start + Duration::from_secs(tick);
        follow_ups.retain(|at| {
            if *at <= now {
                if let Some(k) = keys.follow_up(*at) {
                    note(&k, &mut bound);
                }
                false
            } else {
                true
            }
        });
        let used = keys.send_key(now);
        note(&used, &mut bound);
        if tick % 2 == 0 && tick < 60 {
            // The leaver held what we send now and anything pending.
            let held: Vec<u64> = keys.shareable_for("x").iter().map(|k| k.index).collect();
            let leaver_max = held.into_iter().chain([used.index]).max().unwrap();
            match keys.on_leave(now) {
                LeaveRotation::Rotated(k) => note(&k, &mut bound),
                LeaveRotation::Deferred(at) => follow_ups.push(at),
            }
            let deadline = now + Duration::from_secs(11);
            let mut t = now;
            while keys.send_key(t).index <= leaver_max {
                for at in follow_ups.clone() {
                    if at <= t {
                        if let Some(k) = keys.follow_up(at) {
                            note(&k, &mut bound);
                        }
                        follow_ups.retain(|a| *a != at);
                    }
                }
                t += Duration::from_millis(500);
                assert!(t <= deadline, "a leaver's key in use past 10 s");
            }
            keys.on_join("x", now);
        }
    }
}

#[test]
fn a_participant_gets_on_request_nothing_older_than_its_admission() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new();
    keys.send_key(now);
    assert!(keys.shareable_for("bob").is_empty(), "not admitted");
    let join = keys.on_join("bob", now);
    keys.send_key(now);
    keys.on_join("carol", now);
    let shared: Vec<u64> = keys.shareable_for("bob").iter().map(|k| k.index).collect();
    assert!(shared.iter().all(|i| *i >= join.joiner[0].index));
    keys.forget("bob");
    assert!(keys.shareable_for("bob").is_empty());
}

#[test]
fn receivers_derive_a_later_step_and_keep_it_once_it_opens() {
    let keys = ChannelSenderKeys::new();
    let base = 0x1234 << 8;
    keys.install("alice", base, secret(1));
    let candidates = keys.receive_candidates("alice", 3).unwrap();
    assert_eq!(candidates.len(), 1);
    let derived = sframe::ratchet(&sframe::ratchet(&sframe::ratchet(&secret(1))));
    assert_eq!(candidates[0].index, base + 3);
    assert_eq!(*candidates[0].secret, *derived);
    keys.confirm("alice", candidates.into_iter().next().unwrap());
    assert_eq!(*keys.secret_at("alice", base + 3).unwrap(), *derived);
}

#[test]
fn derivation_stops_at_the_ratchet_bound_and_never_goes_back() {
    let keys = ChannelSenderKeys::new();
    keys.install("alice", 0x10_05, secret(1));
    assert!(
        keys.receive_candidates("alice", 0x05 + 6).is_err(),
        "too far ahead"
    );
    assert!(
        keys.receive_candidates("alice", 0x04).is_err(),
        "no going back"
    );
    assert!(keys.secret_at("alice", 0x10_0b).is_err());
    assert!(keys.secret_at("alice", 0x10_0a).is_ok());
    assert!(
        keys.secret_at("alice", 0x11_05).is_err(),
        "another generation"
    );
}

#[test]
fn candidates_come_from_each_held_generation_newest_first() {
    let keys = ChannelSenderKeys::new();
    keys.install("bob", 0x20_00, secret(1));
    keys.install("bob", 0x21_00, secret(2));
    let c = keys.receive_candidates("bob", 0).unwrap();
    assert_eq!(
        c.iter().map(|c| c.index).collect::<Vec<_>>(),
        [0x21_00, 0x20_00]
    );
}

#[test]
fn received_keys_keep_the_last_five() {
    let keys = ChannelSenderKeys::new();
    for i in 0..7u8 {
        keys.install("alice", (0x1000 + u64::from(i)) << 8, secret(i));
    }
    assert!(keys.secret_at("alice", 0x1006 << 8).is_ok());
    assert!(
        keys.secret_at("alice", 0x1001 << 8).is_err(),
        "the oldest are forgotten"
    );
    keys.forget("alice");
    assert!(keys.secret_at("alice", 0x1006 << 8).is_err());
}

#[test]
fn a_missing_key_names_the_newest_generation_at_that_step() {
    let keys = ChannelSenderKeys::new();
    keys.install("bob", 0x2_05, secret(1));
    assert_eq!(
        keys.receive_candidates("bob", 0x00).err(),
        Some(MissingKey {
            sender: "bob".into(),
            index: 0x2_00
        })
    );
    assert_eq!(
        keys.receive_candidates("carol", 0x09).err().unwrap().index,
        0x09
    );
}

#[test]
fn a_delivery_stays_current_while_the_recipient_can_derive_from_it() {
    let now = Instant::now();
    let keys = ChannelSenderKeys::new();
    let first = keys.send_key(now).index;
    assert!(keys.still_current(first));
    for p in 0..6 {
        keys.send_key(now);
        keys.on_join(&format!("p{p}"), now);
    }
    assert!(!keys.still_current(first), "more than five steps behind");
    let next = rotated(keys.on_leave(now));
    assert!(keys.still_current(next.index), "the pending key");
}

#[test]
fn sessions_start_at_independent_generations() {
    let a = ChannelSenderKeys::new().send_key(Instant::now()).index;
    let b = ChannelSenderKeys::new().send_key(Instant::now()).index;
    // Random 32-bit starts: equal only by a 1-in-2^32 chance.
    assert_ne!(a, b);
    assert_eq!(step(a), 0);
}

/// A participant's session key is learned only under its pseudonym's
/// signature for this channel; nothing is sealed to an unknown one, and a
/// key sealed to ours opens (plan C7.29 G2).
#[test]
fn session_keys_are_learned_only_when_signed_for_this_channel() {
    use ed25519_dalek::{Signer, SigningKey};
    let bob = ChannelSenderKeys::new();
    let bob_pseudonym = SigningKey::from_bytes(&[2; 32]);
    let bob_hex = hex::encode(bob_pseudonym.verifying_key().to_bytes());
    let key = bob.session_public().to_vec();
    let signed = |channel: &str| SignedSessionKey {
        key: key.clone(),
        signature: bob_pseudonym
            .sign(&session::session_key_message("c", channel, &key))
            .to_bytes()
            .to_vec(),
    };

    let alice = ChannelSenderKeys::new();
    assert!(
        alice.seal_for(&bob_hex, b"aad", &[1; 32]).is_none(),
        "unknown"
    );
    assert!(!alice.learn_session_key("c", "ch", &bob_hex, &signed("other")));
    assert!(!alice.learn_session_key("c", "ch", &"00".repeat(32), &signed("ch")));
    assert!(alice.learn_session_key("c", "ch", &bob_hex, &signed("ch")));
    assert_eq!(alice.session_key_of(&bob_hex), Some(signed("ch")));

    let sealed = alice.seal_for(&bob_hex, b"aad", &[1; 32]).unwrap();
    assert_eq!(*bob.open_sealed(b"aad", &sealed).unwrap(), [1; 32]);
    alice.forget(&bob_hex);
    assert!(alice.session_key_of(&bob_hex).is_none());
}
