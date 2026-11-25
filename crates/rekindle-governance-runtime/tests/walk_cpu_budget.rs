#![allow(
    clippy::print_stderr,
    reason = "a perf test whose measurement IS its output; cargo test --nocapture is where it is read"
)]

//! The walk's own work over a 64-head level (plan S7c.1). What gates every
//! run is the work counted in machine-independent units: one signature
//! verification per op (the verdict cache) and the cache's walk steps (one
//! per held op decoded and hash-checked, per readiness check, per dependent
//! visited). The wall-clock target stays as an ignored report, run in
//! release on an idle machine by `cargo xtask perf`: a wall-clock gate
//! failed beside a loaded lib-test run (5447 ms on the Linux machine against
//! a 1000 ms guard, 2026-11-17).

use std::time::{Duration, Instant};

use rekindle_governance_runtime::op_log::held_from_encoded;
use rekindle_governance_runtime::op_walk::walk_with;
use rekindle_governance_runtime::verify_cache::OpVerifyCache;

#[path = "support/walk_dag.rs"]
mod walk_dag;

use walk_dag::{dag, ANCHOR, AUTHORS};

/// Release target for the walk's own CPU over the 130-op DAG: measured
/// 25–29 ms on the Mac (M-series).
const RELEASE_BUDGET_MS: u128 = 100;

/// The walk's steps over the DAG, exact (measured 2026-11-23): any change
/// in the work the walk does is a diff here to explain.
const WALK_STEPS: u64 = 1_160;

/// Signature verifications over the 130-op DAG, exact (measured
/// 2026-11-23): 580, not one per op. An op whose signature passed but
/// whose deps were not yet held is `authentic`, never a kept verdict, so
/// each level's `verify_held_cached` verifies it again.
const VERIFICATIONS: u64 = 580;

/// Walks the DAG with fetches that cost nothing; the cache and the time.
async fn walk_dag() -> (OpVerifyCache, Duration, usize) {
    let dag = dag();
    let dag_ref = &dag;
    let cache = OpVerifyCache::default();
    let started = Instant::now();
    let walked = walk_with(ANCHOR, &dag.heads, &cache, |op_ref| async move {
        Ok(dag_ref
            .encoded(&op_ref)
            .and_then(|e| held_from_encoded(&op_ref, e)))
    })
    .await
    .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(dag.heads.len(), AUTHORS);
    assert_eq!(walked.len(), dag.ops);
    (cache, elapsed, dag.ops)
}

#[tokio::test]
async fn a_sixty_four_head_walk_verifies_each_op_once() {
    let (cache, _, ops) = walk_dag().await;
    let (verifications, steps) = (cache.verifications(), cache.walk_steps());
    eprintln!("[perf] 64-head walk: {ops} ops, {verifications} verifications, {steps} steps");
    assert!(verifications >= ops as u64, "every op verified");
    assert_eq!(verifications, VERIFICATIONS);
    assert_eq!(steps, WALK_STEPS);
}

#[tokio::test]
#[ignore = "wall-clock spec; cargo xtask perf on an idle machine"]
async fn a_sixty_four_head_walk_stays_within_its_cpu_budget() {
    let (_, elapsed, _) = walk_dag().await;
    let ms = elapsed.as_millis();
    eprintln!("[perf] 64-head walk elapsed = {elapsed:?} (target <{RELEASE_BUDGET_MS}ms)");
    assert!(
        ms < RELEASE_BUDGET_MS,
        "64-head walk took {ms} ms, target {RELEASE_BUDGET_MS} ms"
    );
}
