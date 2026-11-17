//! The walk's own CPU over a 64-head level (plan S7c.1), alone in its test
//! binary as `rekindle-governance/tests/perf_targets.rs` is: cargo runs test
//! binaries one after another, so no sibling test loads the CPU while this
//! one measures it (a lib-test run beside 107 others measured 5447 ms on the
//! Linux machine against the 1000 ms debug guard, 2026-11-17).

use std::time::Instant;

use rekindle_governance_runtime::op_log::held_from_encoded;
use rekindle_governance_runtime::op_walk::walk_with;
use rekindle_governance_runtime::verify_cache::OpVerifyCache;

#[path = "support/walk_dag.rs"]
mod walk_dag;

use walk_dag::{dag, ANCHOR, AUTHORS};

/// The walk's own CPU over the 130-op DAG with fetches that cost nothing:
/// decode, hash check and one signature verification per op (the verdict
/// cache) plus one verify pass per level. Release target, asserted when
/// built without debug assertions: measured 25–29 ms on the Mac (M-series),
/// so the Linux test machine (~3× the Mac's time, `perf_targets.rs`) stays
/// under it.
const RELEASE_BUDGET_MS: u128 = 100;

/// The debug guard: measured 181–186 ms on the Mac, so the Linux machine
/// stays under it while a several-fold regression still fails.
const DEBUG_BUDGET_MS: u128 = 1000;

#[tokio::test]
async fn a_sixty_four_head_walk_stays_within_its_cpu_budget() {
    let dag = dag();
    let dag_ref = &dag;
    let started = Instant::now();
    let walked = walk_with(
        ANCHOR,
        &dag.heads,
        &OpVerifyCache::default(),
        |op_ref| async move {
            Ok(dag_ref
                .encoded(&op_ref)
                .and_then(|e| held_from_encoded(&op_ref, e)))
        },
    )
    .await
    .unwrap();
    let ms = started.elapsed().as_millis();
    assert_eq!(dag.heads.len(), AUTHORS);
    assert_eq!(walked.len(), dag.ops);
    let budget = if cfg!(debug_assertions) {
        DEBUG_BUDGET_MS
    } else {
        RELEASE_BUDGET_MS
    };
    assert!(ms < budget, "64-head walk took {ms} ms, budget {budget} ms");
}
