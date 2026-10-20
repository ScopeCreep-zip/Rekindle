//! Loss, reorder and crossing on `SessionRecord`: under random schedules
//! every delivered message decrypts to what was sent, a replay is refused
//! and changes nothing, and lost messages cost nothing but themselves.
//!
//! A message is never held longer than [`MAX_AGE`] steps (it is delivered
//! then), which keeps every schedule inside the receiving-chain bound;
//! the bound itself is tested in `dr::tests`. After the schedule, what is
//! still in flight is delivered and a round trip settles both records on
//! one session.

use proptest::prelude::*;

use super::SessionRecord;
use crate::signal::test_support::{BobKeys, TestRandom};
use crate::signal::RatchetMessage;

const AD: &[u8] = b"ad";
/// The receiver's clock in these tests.
const NOW: i64 = 1_000;

/// Steps a message may stay in flight.
const MAX_AGE: usize = 4;

#[derive(Debug, Clone)]
enum Op {
    Send(bool),
    Deliver(bool, usize),
    Drop(bool, usize),
    Replay(bool, usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => any::<bool>().prop_map(Op::Send),
        3 => (any::<bool>(), any::<usize>()).prop_map(|(b, i)| Op::Deliver(b, i)),
        1 => (any::<bool>(), any::<usize>()).prop_map(|(b, i)| Op::Drop(b, i)),
        1 => (any::<bool>(), any::<usize>()).prop_map(|(b, i)| Op::Replay(b, i)),
    ]
}

/// In flight to one side: `(sent at step, message, plaintext)`.
#[derive(Default)]
struct Wire {
    pending: Vec<(usize, RatchetMessage, Vec<u8>)>,
    delivered: Vec<RatchetMessage>,
}

struct Side {
    record: SessionRecord,
    inbox: Wire,
}

impl Side {
    fn deliver(&mut self, index: usize) {
        let (_, message, plaintext) = self.inbox.pending.remove(index);
        let (record, decrypted) = self
            .record
            .decrypt(
                &message,
                AD,
                NOW,
                &TestRandom::for_message(&message.ciphertext),
            )
            .expect("a delivered message decrypts");
        assert_eq!(decrypted, plaintext);
        self.record = record;
        self.inbox.delivered.push(message);
    }

    /// Send `plaintext` to `to` at `step`.
    fn send(&mut self, to: &mut Self, step: usize, plaintext: &[u8]) {
        let (record, message) = self.record.encrypt(plaintext, AD).unwrap();
        self.record = record;
        to.inbox.pending.push((step, message, plaintext.to_vec()));
    }

    /// Deliver everything still in flight, oldest first.
    fn flush(&mut self) {
        while !self.inbox.pending.is_empty() {
            self.deliver(0);
        }
    }

    /// Deliver, oldest first, whatever has been in flight too long.
    fn deliver_overdue(&mut self, step: usize) {
        while let Some(index) = self
            .inbox
            .pending
            .iter()
            .position(|(sent, _, _)| step - sent >= MAX_AGE)
        {
            self.deliver(index);
        }
    }
}

/// Run `ops` from `start`, then flush and check the records converge.
fn run(mut alice: Side, mut bob: Side, start: usize, ops: &[Op]) {
    let mut counter = 0u32;
    for (step, op) in (start..).zip(ops) {
        alice.deliver_overdue(step);
        bob.deliver_overdue(step);
        match *op {
            Op::Send(from_alice) => {
                let (from, to) = if from_alice {
                    (&mut alice, &mut bob)
                } else {
                    (&mut bob, &mut alice)
                };
                counter += 1;
                from.send(to, step, &counter.to_be_bytes());
            }
            Op::Deliver(to_bob, pick) | Op::Drop(to_bob, pick) | Op::Replay(to_bob, pick) => {
                let side = if to_bob { &mut bob } else { &mut alice };
                if let Op::Replay(..) = op {
                    if let Some(message) = pick_from(&side.inbox.delivered, pick) {
                        let before = side.record.to_bytes();
                        let replayed = side.record.decrypt(message, AD, NOW, &TestRandom::new(1));
                        assert!(replayed.is_err(), "a replay decrypted");
                        assert_eq!(*before, *side.record.to_bytes());
                    }
                    continue;
                }
                if side.inbox.pending.is_empty() {
                    continue;
                }
                let index = pick % side.inbox.pending.len();
                if let Op::Deliver(..) = op {
                    side.deliver(index);
                } else {
                    side.inbox.pending.remove(index);
                }
            }
        }
    }
    converge(&mut alice, &mut bob);
}

/// Flush both inboxes, then a round trip: both records end on the same
/// active session.
fn converge(alice: &mut Side, bob: &mut Side) {
    alice.flush();
    bob.flush();
    alice.send(bob, 0, b"ping");
    bob.flush();
    bob.send(alice, 0, b"pong");
    alice.flush();
    assert_eq!(
        alice.record.active.as_ref().unwrap().base_key(),
        bob.record.active.as_ref().unwrap().base_key()
    );
}

fn pick_from<T>(items: &[T], pick: usize) -> Option<&T> {
    (!items.is_empty()).then(|| &items[pick % items.len()])
}

fn side(record: SessionRecord) -> Side {
    Side {
        record,
        inbox: Wire::default(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// One initiator; messages lost and reordered both ways.
    #[test]
    fn loss_and_reorder(one_time in any::<bool>(), ops in prop::collection::vec(op(), 1..60)) {
        let a_keys = BobKeys::new();
        let b_keys = BobKeys::new();
        let (alice, init) = SessionRecord::new(b_keys.identity.public_key_bytes())
            .initiate(&a_keys.identity, &b_keys.bundle(one_time), b"init", AD, &TestRandom::new(1))
            .unwrap();
        let (bob, first) = SessionRecord::new(a_keys.identity.public_key_bytes())
            .insert_from_init(
                &init,
                &b_keys.keys_for(init.one_time_prekey_id, init.pq_prekey),
                AD,
                NOW,
                &TestRandom::for_message(&init.message.ciphertext),
            )
            .unwrap();
        prop_assert_eq!(first, b"init".to_vec());
        run(side(alice), side(bob), 0, &ops);
    }

    /// Both sides initiate at once and the inits cross: the side that
    /// hears first (`a_first`) accepts and replies on the peer's session;
    /// the other sends once more on its own session, and only then accepts
    /// the first side's init. Those two messages stay in flight into the
    /// random schedule, so each lands on a record whose active session has
    /// moved on.
    #[test]
    fn crossing(a_first in any::<bool>(), ops in prop::collection::vec(op(), 1..60)) {
        let a_keys = BobKeys::new();
        let b_keys = BobKeys::new();
        let (a_record, a_init) = SessionRecord::new(b_keys.identity.public_key_bytes())
            .initiate(&a_keys.identity, &b_keys.bundle(true), b"a", AD, &TestRandom::new(3))
            .unwrap();
        let (b_record, b_init) = SessionRecord::new(a_keys.identity.public_key_bytes())
            .initiate(&b_keys.identity, &a_keys.bundle(true), b"b", AD, &TestRandom::new(4))
            .unwrap();
        let mut a = (side(a_record), &a_keys, a_init);
        let mut b = (side(b_record), &b_keys, b_init);
        let (first, second) = if a_first { (&mut a, &mut b) } else { (&mut b, &mut a) };

        let from_second = accept(&mut first.0, first.1, &second.2);
        prop_assert_eq!(from_second, if a_first { b"b".to_vec() } else { b"a".to_vec() });
        first.0.send(&mut second.0, 0, b"reply on the peer's session");
        second.0.send(&mut first.0, 0, b"again on our own session");
        let from_first = accept(&mut second.0, second.1, &first.2);
        prop_assert_eq!(from_first, if a_first { b"a".to_vec() } else { b"b".to_vec() });

        run(a.0, b.0, 1, &ops);
    }
}

/// Accept `init` into `side`'s record, returning the first plaintext.
fn accept(side: &mut Side, keys: &BobKeys, init: &super::PreKeyInit) -> Vec<u8> {
    let (record, first) = side
        .record
        .insert_from_init(
            init,
            &keys.keys_for(init.one_time_prekey_id, init.pq_prekey),
            AD,
            NOW,
            &TestRandom::for_message(&init.message.ciphertext),
        )
        .unwrap();
    side.record = record;
    first
}
