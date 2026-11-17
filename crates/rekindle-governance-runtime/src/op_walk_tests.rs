//! The walk's cost (plan S7c.1): one read per author log per walk, at most
//! sixteen fetches in flight per level, and a 64-head level's CPU cost.
//!
//! The DAG: the creator's two ops, then 64 authors with two ops each, the
//! first citing the creator's second op: a 64-head level twice over (the
//! shallow-join shape, S7c.0), walked from the 64 heads.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::fixture::meta;
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::GovernanceOp;
use rekindle_types::id::PseudonymKey;

use super::*;
use crate::op_log::held_from_encoded;

const ANCHOR: &str = "A";
const AUTHORS: usize = 64;

/// Each author record's op log; chain seq = index + 1.
struct Dag {
    logs: HashMap<String, Vec<Vec<u8>>>,
    heads: Vec<OpRef>,
    ops: usize,
}

impl Dag {
    fn encoded(&self, op_ref: &OpRef) -> Option<Vec<u8>> {
        let index = usize::try_from(op_ref.author_seq).ok()?;
        self.logs.get(&op_ref.author_record)?.get(index).cloned()
    }

    fn read_after(&self, record: &str, cursor: u64) -> LogRead {
        let log = self.logs.get(record).cloned().unwrap_or_default();
        LogRead {
            entries: (1u64..)
                .zip(log.iter().cloned())
                .filter(|(seq, _)| *seq > cursor)
                .collect(),
            last_seq: log.len() as u64,
        }
    }
}

/// An author's chain of `len` ops in `record`, the first citing `first_deps`.
fn chain(seed: u8, record: &str, len: u64, first_deps: &[OpRef]) -> (Vec<Vec<u8>>, OpRef) {
    let key = SigningKey::from_bytes(&[seed; 32]);
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let mut log = Vec::new();
    let mut last: Option<OpRef> = None;
    for seq in 0..len {
        let deps = last
            .clone()
            .map_or_else(|| first_deps.to_vec(), |prev| vec![prev]);
        let mut op = GovernanceOp {
            author: author.clone(),
            seq,
            prev: last.as_ref().map(|r| r.op_hash),
            deps,
            issued_at: 10 + seq,
            body: meta("m"),
            sig: [0; 64],
        };
        let unsigned = op_unsigned_bytes(&op);
        op.sig = key.sign(&op_signing_bytes(ANCHOR, &unsigned)).to_bytes();
        let hash = op_hash(&unsigned, &op.sig);
        log.push(encode_op(&op).unwrap());
        last = Some(OpRef {
            op_hash: hash,
            author: author.clone(),
            author_record: record.to_owned(),
            author_seq: seq,
        });
    }
    (log, last.unwrap())
}

fn dag() -> Dag {
    let mut logs = HashMap::new();
    let (creator_log, creator_head) = chain(1, "rec-creator", 2, &[]);
    logs.insert("rec-creator".to_owned(), creator_log);
    let mut heads = Vec::new();
    for i in 0..AUTHORS {
        let record = format!("rec-{i}");
        let seed = u8::try_from(i + 2).unwrap();
        let (log, head) = chain(seed, &record, 2, std::slice::from_ref(&creator_head));
        logs.insert(record, log);
        heads.push(head);
    }
    Dag {
        ops: 2 + 2 * AUTHORS,
        logs,
        heads,
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
                        Ok::<_, GovernanceRuntimeError>(dag_ref.read_after(&record, cursor))
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
    assert_eq!(walked.len(), dag.ops);
    let budget = if cfg!(debug_assertions) {
        DEBUG_BUDGET_MS
    } else {
        RELEASE_BUDGET_MS
    };
    assert!(ms < budget, "64-head walk took {ms} ms, budget {budget} ms");
}
