//! The dependency-order walk of `verify_held_cached` (plan E3.2 S7c,
//! S7c.1's cost targets), alone in its test binary as `walk_cpu_budget.rs`
//! is, so no sibling test loads the CPU while this one measures it.
//!
//! Every `rebuild_ops` / `verify_held_with` caller (a rebuild, the sweep,
//! the bootstrap reply) reads the whole held set through the community's
//! verdict cache, so with every verdict kept the read is the walk alone.
//! It was one pass over the waiting ops per DAG level: a single-author
//! chain of n ops is n levels, n²/2 readiness checks (10,000 ops: ~60 s on
//! a Mac debug build). It is a ready queue now (Kahn's algorithm), linear
//! in the ops and their deps.

use std::time::{Duration, Instant};

use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::fixture::meta;
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_governance_runtime::op_log::{held_from_encoded, HeldOp};
use rekindle_governance_runtime::verify_cache::{verify_held_cached, OpVerifyCache};
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{GovernanceOp, OpRef};
use rekindle_types::id::PseudonymKey;

const ANCHOR: &str = "A";
const RECORD: &str = "rec-creator";

/// The bound on a warm read of the 10,000-op chain: the 5 s callee answer
/// timeout (`rpc_app_call.rs:131-138`) the bootstrap reply must meet (plan
/// S7c.6 (5)); S7c.1's own budgets are for the 130-op walk DAG and none
/// fits 10,000 ops. Measured on the Mac (M-series, debug): 69.2 s before
/// the ready queue, 0.43 s after.
const TEN_K_BUDGET: Duration = Duration::from_secs(5);

/// One author's signed chain of `len` ops, each citing the one before.
fn chain(len: u64) -> Vec<HeldOp> {
    let key = SigningKey::from_bytes(&[3; 32]);
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let mut held = Vec::new();
    let mut last: Option<OpRef> = None;
    for seq in 0..len {
        let mut op = GovernanceOp {
            author: author.clone(),
            seq,
            prev: last.as_ref().map(|r| r.op_hash),
            deps: last.clone().into_iter().collect(),
            issued_at: 10 + seq,
            body: meta("m"),
            sig: [0; 64],
        };
        let unsigned = op_unsigned_bytes(&op);
        op.sig = key.sign(&op_signing_bytes(ANCHOR, &unsigned)).to_bytes();
        let op_ref = OpRef {
            op_hash: op_hash(&unsigned, &op.sig),
            author: author.clone(),
            author_record: RECORD.to_owned(),
            author_seq: seq,
        };
        held.push(held_from_encoded(&op_ref, encode_op(&op).unwrap()).unwrap());
        last = Some(op_ref);
    }
    held
}

/// A warm read of `held`: every verdict kept by a first read.
fn warm_read(held: &[HeldOp]) -> Duration {
    let cache = OpVerifyCache::default();
    let cold = verify_held_cached(ANCHOR, held, &cache);
    assert_eq!(cold.ops.len(), held.len());
    let verifications = cache.verifications();
    let started = Instant::now();
    let warm = verify_held_cached(ANCHOR, held, &cache);
    let elapsed = started.elapsed();
    assert_eq!(cache.verifications(), verifications, "every verdict kept");
    assert_eq!(warm.ops, cold.ops);
    assert!(warm.pending.is_empty() && warm.quarantined.is_empty());
    elapsed
}

/// The fastest of three warm reads, so a scheduler hiccup does not count.
fn best_warm_read(held: &[HeldOp]) -> Duration {
    (0..3).map(|_| warm_read(held)).min().unwrap()
}

#[test]
fn a_warm_read_of_a_ten_thousand_op_chain_meets_the_callee_window() {
    let held = chain(10_000);
    let elapsed = warm_read(&held);
    assert!(
        elapsed < TEN_K_BUDGET,
        "10,000-op warm read took {elapsed:?}, budget {TEN_K_BUDGET:?}"
    );
}

#[test]
fn a_warm_read_grows_linearly_with_the_chain() {
    let times: Vec<(u64, Duration)> = [1_000, 2_000, 4_000]
        .into_iter()
        .map(|len| (len, best_warm_read(&chain(len))))
        .collect();
    let (two_k, four_k) = (times[1].1, times[2].1);
    // Doubling the chain at most ~doubles the read (4× when quadratic).
    // Measured on the Mac (debug): 0.53 / 2.07 / 8.33 s before the ready
    // queue, 44 / 85 / 164 ms after.
    assert!(
        four_k.as_secs_f64() < 2.5 * two_k.as_secs_f64(),
        "warm reads {times:?}: 4,000 ops took more than 2.5× 2,000"
    );
}
