//! Differential tests: [`SessionState`] against the §3.5 reference model
//! over random schedules of sends, deliveries in any order, losses and
//! replays. Both sides draw the same ratchet keys (one seed per step), so
//! every ciphertext must match byte for byte and every decryption must
//! agree.

use proptest::prelude::*;

use super::model::Model;
use super::{bob_spk_public, pair, AD, BOB_SPK, SK};
use crate::signal::dr::{RatchetMessage, SessionState};
use crate::signal::test_support::TestRandom;

#[derive(Debug, Clone)]
enum Op {
    /// The initiator (`true`) or the responder sends.
    Send(bool),
    /// Deliver a pending message to the responder (`true`) or initiator.
    Deliver(bool, usize),
    /// Lose a pending message.
    Drop(bool, usize),
    /// Deliver an already delivered message again.
    Replay(bool, usize),
    /// Hold back a long run: the initiator sends this many at once.
    Burst(u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => any::<bool>().prop_map(Op::Send),
        4 => (any::<bool>(), any::<usize>()).prop_map(|(b, i)| Op::Deliver(b, i)),
        1 => (any::<bool>(), any::<usize>()).prop_map(|(b, i)| Op::Drop(b, i)),
        1 => (any::<bool>(), any::<usize>()).prop_map(|(b, i)| Op::Replay(b, i)),
        1 => (1u8..40).prop_map(Op::Burst),
    ]
}

/// One party, run by the implementation and by the model in step.
struct Party {
    real: SessionState,
    model: Model,
}

impl Party {
    fn send(&mut self, plaintext: &[u8]) -> Option<RatchetMessage> {
        let real = self.real.encrypt(plaintext, b"ad").ok();
        let model = self.model.encrypt(plaintext, b"ad");
        assert_eq!(real, model, "encrypt diverged");
        real
    }

    fn receive(&mut self, message: &RatchetMessage, seed: u64) -> Option<Vec<u8>> {
        let mut trial = self.real.clone();
        let real = trial.decrypt(message, b"ad", &TestRandom::new(seed)).ok();
        if real.is_some() {
            self.real = trial;
        }
        let model = self.model.decrypt(message, b"ad", &TestRandom::new(seed));
        assert_eq!(real, model, "decrypt diverged");
        real
    }
}

/// Messages in flight to one side, and those already delivered there.
#[derive(Default)]
struct Wire {
    pending: Vec<(RatchetMessage, Vec<u8>)>,
    delivered: Vec<(RatchetMessage, Vec<u8>)>,
}

fn parties(seed: u64) -> (Party, Party) {
    let (alice, bob) = pair(seed);
    let alice_model = Model::alice(&SK, &AD, bob_spk_public(), &TestRandom::new(seed));
    let bob_model = Model::bob(&SK, &AD, BOB_SPK);
    (
        Party {
            real: alice,
            model: alice_model,
        },
        Party {
            real: bob,
            model: bob_model,
        },
    )
}

fn run(seed: u64, ops: &[Op]) {
    let (mut alice, mut bob) = parties(seed);
    let mut to_bob = Wire::default();
    let mut to_alice = Wire::default();
    let mut counter = 0u32;
    for (step, op) in ops.iter().enumerate() {
        let step_seed = seed
            .wrapping_mul(1_000_003)
            .wrapping_add(u64::try_from(step).expect("a step index fits u64"));
        let mut send = |from: &mut Party, wire: &mut Wire| {
            counter += 1;
            let plaintext = counter.to_be_bytes().to_vec();
            if let Some(message) = from.send(&plaintext) {
                wire.pending.push((message, plaintext));
            }
        };
        match *op {
            Op::Send(true) => send(&mut alice, &mut to_bob),
            Op::Send(false) => send(&mut bob, &mut to_alice),
            Op::Burst(count) => (0..count).for_each(|_| send(&mut alice, &mut to_bob)),
            Op::Deliver(to_b, pick) | Op::Drop(to_b, pick) | Op::Replay(to_b, pick) => {
                let (party, wire) = if to_b {
                    (&mut bob, &mut to_bob)
                } else {
                    (&mut alice, &mut to_alice)
                };
                let replay = matches!(op, Op::Replay(..));
                let from = if replay {
                    &wire.delivered
                } else {
                    &wire.pending
                };
                if from.is_empty() {
                    continue;
                }
                let index = pick % from.len();
                if replay {
                    let (message, _) = wire.delivered[index].clone();
                    assert_eq!(
                        party.receive(&message, step_seed),
                        None,
                        "a replay decrypted"
                    );
                    continue;
                }
                let (message, plaintext) = wire.pending.remove(index);
                if matches!(op, Op::Deliver(..)) {
                    if let Some(decrypted) = party.receive(&message, step_seed) {
                        assert_eq!(decrypted, plaintext);
                        wire.delivered.push((message, plaintext));
                    }
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn the_ratchet_matches_the_reference_model(seed in any::<u64>(), ops in prop::collection::vec(op(), 1..80)) {
        run(seed, &ops);
    }
}

/// A jump that overflows a chain's skipped-key bound, then the dropped
/// and kept keys, agree with the model's deletion policy.
#[test]
fn a_long_skip_matches_the_reference_model() {
    let mut ops = vec![Op::Burst(39); 60];
    ops.push(Op::Deliver(true, 60 * 39 - 1));
    ops.extend((0..30).map(|i| Op::Deliver(true, i * 77)));
    run(11, &ops);
}
