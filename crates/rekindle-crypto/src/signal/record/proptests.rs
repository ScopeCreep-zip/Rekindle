//! Loss, reorder and crossing on `SessionRecord`: under random schedules
//! every delivered message decrypts to what was sent, a replay is refused
//! and changes nothing, and lost messages cost nothing but themselves.
//!
//! A message is never held longer than [`MAX_AGE`] steps (it is delivered
//! then), which keeps every schedule inside the receiving-chain bound;
//! the bound itself is tested in `dr::tests`.

use proptest::prelude::*;

use super::SessionRecord;
use crate::signal::test_support::{BobKeys, TestRandom};
use crate::signal::RatchetMessage;

const AD: &[u8] = b"ad";

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
            .decrypt(&message, AD, &TestRandom::for_message(&message.ciphertext))
            .expect("a delivered message decrypts");
        assert_eq!(decrypted, plaintext);
        self.record = record;
        self.inbox.delivered.push(message);
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

fn run(mut alice: Side, mut bob: Side, ops: &[Op]) {
    let mut counter = 0u32;
    for (step, op) in ops.iter().enumerate() {
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
                let plaintext = counter.to_be_bytes().to_vec();
                let (record, message) = from.record.encrypt(&plaintext, AD).unwrap();
                from.record = record;
                to.inbox.pending.push((step, message, plaintext));
            }
            Op::Deliver(to_bob, pick) | Op::Drop(to_bob, pick) | Op::Replay(to_bob, pick) => {
                let side = if to_bob { &mut bob } else { &mut alice };
                if let Op::Replay(..) = op {
                    if let Some(message) = pick_from(&side.inbox.delivered, pick) {
                        let before = side.record.to_bytes();
                        let replayed = side.record.decrypt(message, AD, &TestRandom::new(1));
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
                &TestRandom::for_message(&init.message.ciphertext),
            )
            .unwrap();
        prop_assert_eq!(first, b"init".to_vec());
        run(side(alice), side(bob), &ops);
    }

    /// Both sides initiate at once, the inits arrive in either order, then
    /// the same random schedule.
    #[test]
    fn crossing(a_first in any::<bool>(), ops in prop::collection::vec(op(), 1..60)) {
        let a_keys = BobKeys::new();
        let b_keys = BobKeys::new();
        let a_record = SessionRecord::new(b_keys.identity.public_key_bytes());
        let b_record = SessionRecord::new(a_keys.identity.public_key_bytes());
        let (a_record, a_init) = a_record
            .initiate(&a_keys.identity, &b_keys.bundle(true), b"a", AD, &TestRandom::new(3))
            .unwrap();
        let (b_record, b_init) = b_record
            .initiate(&b_keys.identity, &a_keys.bundle(true), b"b", AD, &TestRandom::new(4))
            .unwrap();
        let accept = |record: &SessionRecord, keys: &BobKeys, init: &super::PreKeyInit| {
            record
                .insert_from_init(
                    init,
                    &keys.keys_for(init.one_time_prekey_id, init.pq_prekey),
                    AD,
                    &TestRandom::for_message(&init.message.ciphertext),
                )
                .unwrap()
                .0
        };
        let (a_record, b_record) = if a_first {
            let b_record = accept(&b_record, &b_keys, &a_init);
            (accept(&a_record, &a_keys, &b_init), b_record)
        } else {
            let a_record = accept(&a_record, &a_keys, &b_init);
            (a_record, accept(&b_record, &b_keys, &a_init))
        };
        run(side(a_record), side(b_record), &ops);
    }
}
