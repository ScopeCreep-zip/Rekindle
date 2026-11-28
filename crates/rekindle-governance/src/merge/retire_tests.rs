//! Several `DeviceRetired` for one key (ADR 0022 rule 5, lead decision K4b
//! Q2, Reading A): the lowest *effective* `lastSeq` over every
//! `DeviceRetired` and every carrier of it counts, a witness of another
//! key lifts it, and a resume the snapshot cannot decide defers.

use rekindle_types::governance::{AdmissionMode, GenesisCert};

use crate::device_judge::KeyRefusal;
use crate::fixture::{cert, meta, op, op_on};
use crate::merge::device_cp_tests::{device_op, joined_with_device, who};
use crate::merge::op_proptests::{joined, pk};
use crate::merge::resume_tests::split;
use crate::merge::{merge_from, merge_outcome, MergeOutcome, NotOnTop, QuarantineReason};
use crate::op::VerifiedOp;

/// Member 8's first device key.
const D: u8 = 3;

/// The creator's genesis, member 8's join, its key `d`'s ops at seqs
/// 0..=10 (each citing only the one before; no other key cites them), and
/// two retirements of `d`: R2 signed 5, its carrier citing `d@5`; R1 signed
/// 3, its carrier citing `d@10` (effective lastSeq 10).
struct Case {
    c0: VerifiedOp,
    j: VerifiedOp,
    d: Vec<VerifiedOp>,
    r2: VerifiedOp,
    r1: VerifiedOp,
}

impl Case {
    fn new() -> Self {
        let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
        let j = op(&who(8), 0, &[&c0], 1, joined_with_device(8, D));
        let mut d: Vec<VerifiedOp> = Vec::new();
        for seq in 0..=10u64 {
            let prev = d.last().unwrap_or(&j).clone();
            d.push(op_on(&who(8), [D; 32], seq, &[&prev], 10 + seq, meta("d")));
        }
        let r2 = op(&who(8), 1, &[&j, &d[5]], 30, device_op(8, 1, D, 5, 8));
        let r1 = op(&who(8), 2, &[&r2, &d[10]], 31, device_op(8, 1, D, 3, 8));
        Self { c0, j, d, r2, r1 }
    }

    fn all(&self) -> Vec<VerifiedOp> {
        let mut all = vec![self.c0.clone(), self.j.clone()];
        all.extend(self.d.iter().cloned());
        all.extend([self.r2.clone(), self.r1.clone()]);
        all
    }
}

fn c() -> GenesisCert {
    cert(&pk(1), AdmissionMode::Open)
}

/// `d`'s ops at or below `last` stand (never quarantined nor pending; the
/// key is effective, member 8's `CommunityMeta` is refused on rank), and
/// every one above is quarantined "above a retirement".
fn assert_retired_above(out: &MergeOutcome, d: &[VerifiedOp], last: usize) {
    let retired = QuarantineReason::DeviceKey {
        refusal: KeyRefusal::Retired,
    };
    for (seq, o) in d.iter().enumerate() {
        let quarantined = out.quarantined.iter().find(|(h, _)| *h == o.hash());
        assert!(!out.pending.contains(&o.hash()), "d@{seq} pending");
        if seq <= last {
            assert_eq!(quarantined, None, "d@{seq} stands");
        } else {
            assert_eq!(quarantined.map(|(_, r)| r), Some(&retired), "d@{seq}");
        }
    }
}

#[test]
fn the_lowest_effective_last_seq_counts_not_the_lowest_signed() {
    let k = Case::new();
    let mut all = k.all();
    let out = merge_outcome(&c(), &all);
    assert_retired_above(&out, &k.d, 5);
    all.reverse();
    let reversed = merge_outcome(&c(), &all);
    assert_retired_above(&reversed, &k.d, 5);
    assert_eq!(reversed.state, out.state, "order-free");
    // A third carrier of R1 with genesis-only deps saw nothing of `d`: R1's
    // floor stays 10, so nothing changes.
    let r1b = op(&who(8), 3, &[&k.c0], 32, device_op(8, 1, D, 3, 8));
    all.push(r1b.clone());
    let more = merge_outcome(&c(), &all);
    assert!(
        !more.pending.contains(&r1b.hash()),
        "the third carrier is held"
    );
    assert_retired_above(&more, &k.d, 5);
    // Another key's op citing `d@8` witnesses it: 6..=8 move back, 9 and
    // 10 stay quarantined (the one reverse move).
    let w = op(&pk(1), 1, &[&k.c0, &k.d[8]], 33, meta("w"));
    all.push(w.clone());
    let witnessed = merge_outcome(&c(), &all);
    assert!(!witnessed.pending.contains(&w.hash()));
    assert_retired_above(&witnessed, &k.d, 8);
    all.reverse();
    assert_retired_above(&merge_outcome(&c(), &all), &k.d, 8);
}

/// A resume decides only what the snapshot decides: an op of a retired key
/// above its floor, or any op while the cut holds a retired key's ops above
/// its floor (it may witness them), answers `DeviceJudged`; with no such
/// op, the resume equals the full merge.
#[test]
fn a_resume_past_a_retirement_defers_what_the_snapshot_cannot_decide() {
    let k = Case::new();
    // R2 alone in the cut, `d`'s ops above it beside: `d@6` is retired.
    let mut held: Vec<VerifiedOp> = vec![k.c0.clone(), k.j.clone()];
    held.extend(k.d.iter().cloned());
    held.push(k.r2.clone());
    let full = merge_outcome(&c(), &held);
    assert_retired_above(&full, &k.d, 5);
    let (snap, power, rest) = split(&c(), &held, &[&k.r2]);
    assert!(matches!(
        merge_from(&c(), &snap, &power, &rest),
        Err(NotOnTop::DeviceJudged(h)) if h == k.d[6].hash()
    ));
    // Both retirements in the cut with `d@6..=10`: an op on top cites R1,
    // so it witnesses `d@10`; the snapshot cannot say, so it defers.
    let mut all = k.all();
    let x = op(&pk(1), 1, &[&k.c0, &k.r1], 40, meta("x"));
    all.push(x.clone());
    let (snap, power, rest) = split(&c(), &all, &[&k.r1]);
    assert!(matches!(
        merge_from(&c(), &snap, &power, &rest),
        Err(NotOnTop::DeviceJudged(h)) if h == x.hash()
    ));
    assert_retired_above(&merge_outcome(&c(), &all), &k.d, 10);
    // `d` holds nothing above R2 in the cut: an op on top resumes and
    // equals the full merge.
    let mut short: Vec<VerifiedOp> = vec![k.c0.clone(), k.j.clone()];
    short.extend(k.d[..=5].iter().cloned());
    short.push(k.r2.clone());
    let y = op(&pk(1), 1, &[&k.c0, &k.r2], 41, meta("y"));
    short.push(y);
    let full = merge_outcome(&c(), &short);
    let (snap, power, rest) = split(&c(), &short, &[&k.r2]);
    let resumed = merge_from(&c(), &snap, &power, &rest).expect("decided by the snapshot");
    assert_eq!(resumed.state, full.state);
}

/// The retired key's own copy cannot lift its floor by carrying a
/// retirement in an op of its own that cites its later ops (ADR 0022 rule
/// 5, pass 3: "the key's own ops are left out, or a copy's own chain would
/// lift the floor over everything it wrote"): only R2 (signed 5) holds.
#[test]
fn a_retired_keys_own_carrier_lifts_no_floor() {
    let k = Case::new();
    // `d` itself carries R2's element, citing its own `d@10`.
    let thief = op_on(
        &who(8),
        [D; 32],
        11,
        &[&k.d[10]],
        40,
        device_op(8, 1, D, 5, 8),
    );
    let mut all = vec![k.c0.clone(), k.j.clone()];
    all.extend(k.d.iter().cloned());
    all.extend([k.r2.clone(), thief]);
    let out = merge_outcome(&c(), &all);
    assert_retired_above(&out, &k.d, 5);
}
