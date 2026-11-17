//! The walk's cost (plan S7c.1): one read per author log per walk and at
//! most sixteen fetches in flight per level. The 64-head level's CPU budget
//! runs alone in its own test binary (`tests/walk_cpu_budget.rs`), so the
//! other tests here never load the CPU it measures.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::op_log::held_from_encoded;

#[path = "../tests/support/walk_dag.rs"]
mod walk_dag;

use walk_dag::{dag, Dag, ANCHOR, AUTHORS};

/// `record`'s log entries after `cursor`, as one author-log read returns them.
fn read_after(dag: &Dag, record: &str, cursor: u64) -> LogRead {
    let log = dag.logs.get(record).cloned().unwrap_or_default();
    LogRead {
        entries: (1u64..)
            .zip(log.iter().cloned())
            .filter(|(seq, _)| *seq > cursor)
            .collect(),
        last_seq: log.len() as u64,
    }
}

#[tokio::test]
async fn a_walk_reads_each_author_log_once() {
    let dag = dag();
    let reads: parking_lot::Mutex<HashMap<String, usize>> = parking_lot::Mutex::default();
    let logs = WalkLogCache::default();
    let (dag_ref, reads_ref, logs_ref) = (&dag, &reads, &logs);
    let walked = walk_with(
        ANCHOR,
        &dag.heads,
        &OpVerifyCache::default(),
        |op_ref| async move {
            let record = op_ref.author_record.clone();
            let encoded = logs_ref
                .entry(
                    &op_ref.author_record,
                    op_ref.author_seq + 1,
                    |cursor| async move {
                        *reads_ref.lock().entry(record.clone()).or_default() += 1;
                        Ok::<_, GovernanceRuntimeError>(read_after(dag_ref, &record, cursor))
                    },
                )
                .await?;
            Ok(encoded.and_then(|e| held_from_encoded(&op_ref, e)))
        },
    )
    .await
    .unwrap();
    assert_eq!(walked.len(), dag.ops);
    let reads = reads.into_inner();
    assert_eq!(reads.len(), AUTHORS + 1);
    assert!(
        reads.values().all(|n| *n == 1),
        "each author log is read once per walk: {reads:?}"
    );
}

/// One fetch's simulated round trip.
const RTT: Duration = Duration::from_millis(50);

#[tokio::test(start_paused = true)]
async fn a_walk_level_holds_at_most_sixteen_fetches_in_flight() {
    let dag = dag();
    let in_flight = AtomicUsize::new(0);
    let most = AtomicUsize::new(0);
    let (dag_ref, in_flight_ref, most_ref) = (&dag, &in_flight, &most);
    let started = tokio::time::Instant::now();
    let walked = walk_with(
        ANCHOR,
        &dag.heads,
        &OpVerifyCache::default(),
        |op_ref| async move {
            let now = in_flight_ref.fetch_add(1, Ordering::SeqCst) + 1;
            most_ref.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(RTT).await;
            in_flight_ref.fetch_sub(1, Ordering::SeqCst);
            Ok(dag_ref
                .encoded(&op_ref)
                .and_then(|e| held_from_encoded(&op_ref, e)))
        },
    )
    .await
    .unwrap();
    assert_eq!(walked.len(), dag.ops);
    assert_eq!(most.load(Ordering::SeqCst), WALK_FETCHES_IN_FLIGHT);
    // The levels hold 64, 64, 1 and 1 refs: 4 + 4 + 1 + 1 round trips at
    // sixteen in flight, where one fetch at a time took 130.
    let rounds = 2 * AUTHORS.div_ceil(WALK_FETCHES_IN_FLIGHT) + 2;
    let bound = RTT * u32::try_from(rounds).unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= bound && elapsed < bound + RTT,
        "{elapsed:?} vs {bound:?}"
    );
}
