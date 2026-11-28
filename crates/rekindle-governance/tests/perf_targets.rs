#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a perf test whose measurement IS its output; cargo test --nocapture is where it is read"
)]

//! Architecture §32 Phase 7 / Week 26 performance targets.
//!
//! The spec (communities architecture §32): "CRDT merge: target <10ms for
//! 10,000 governance entries". What gates every run is the merge's work,
//! counted in machine-independent units (`fixture::counts`): a wall-clock
//! gate fails on a loaded machine and passes a regression on a fast one
//! (Criterion's FAQ: do not gate CI on wall clock; rust-analyzer gates its
//! incrementality on executed-query counts). The wall-clock spec stays as
//! an ignored report, run in release on an idle machine by `cargo xtask perf`.

use rekindle_governance::fixture::{at, cert, counts, ops_in_order, MergeCounts, Stamped};
use rekindle_governance::merge::merge;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry};
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions;

const ENTRY_COUNT: usize = 10_000;

/// The spec target, reported by the ignored wall-clock test in release.
const RELEASE_BUDGET_MS: u128 = 10;

/// The merge's work at 10,000 entries, exact (measured 2026-11-23): any
/// change in the work the merge does is a diff here to explain. PF1
/// (2026-11-24): each op's anchor comes from its deps' anchors
/// (`anchor_steps`, one per dep), no longer from an ancestry query per op,
/// which built every row (was `contains` 10,000, `rows_built` 10,000,
/// `words_ored` 785,942).
const WORK_AT_10K: MergeCounts = MergeCounts {
    rounds: 1,
    try_apply: 10_000,
    contains: 0,
    rows_built: 0,
    words_ored: 0,
    frontier_steps: 0,
    anchor_steps: 19_947,
};

/// The `stateRoot` preimage of the 10,000 entry community, exact (the
/// 736,246 B the S7c.8 note measured predates later state fields).
const PREIMAGE_AT_10K: usize = 1_059_489;

fn gen_synthetic_entries(n: usize) -> Vec<(PseudonymKey, Vec<Stamped>)> {
    let mut by_author: std::collections::HashMap<PseudonymKey, Vec<Stamped>> =
        std::collections::HashMap::new();
    let creator = PseudonymKey([0x42u8; 32]);

    // Genesis: creator at lamport 1, role at 2.
    let creator_entries = vec![
        at(
            1,
            GovernanceEntry::CommunityMeta {
                name: Some("Perf Test Community".to_string()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
        ),
        at(
            2,
            GovernanceEntry::RoleDefinition {
                role_id: rekindle_types::id::RoleId([0u8; 16]),
                name: "@everyone".into(),
                permissions: permissions::SEND_MESSAGES | permissions::READ_HISTORY,
                position: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        ),
    ];
    by_author.insert(creator.clone(), creator_entries);

    // Synthesize n-2 plain RoleAssignment entries, distributed across
    // 50 authors with monotonically increasing lamport timestamps.
    let mut lamport: u64 = 3;
    for i in 0..(n - 2) {
        let author_byte = u8::try_from(i % 50).unwrap_or(0).wrapping_add(1);
        let author = PseudonymKey([author_byte; 32]);
        let target = PseudonymKey([author_byte; 32]);
        let entry = at(
            lamport,
            GovernanceEntry::RoleAssignment {
                target,
                role_id: rekindle_types::id::RoleId([0u8; 16]),
            },
        );
        by_author.entry(author).or_default().push(entry);
        lamport += 1;
    }

    by_author.into_iter().collect()
}

fn merge_work(n: usize) -> MergeCounts {
    let creator = PseudonymKey([0x42u8; 32]);
    let ops = ops_in_order(&gen_synthetic_entries(n));
    let cert = cert(&creator, AdmissionMode::default());
    counts(|| {
        let _ = merge(&cert, &ops);
    })
}

/// `next` is at most ~2× `prev` (5% slack): the work at twice the entries.
fn at_most_doubles(prev: u64, next: u64) -> bool {
    20 * next <= 42 * prev
}

#[test]
fn merge_work_grows_linearly_in_the_entries() {
    let sizes = [2_500, 5_000, ENTRY_COUNT];
    let work: Vec<MergeCounts> = sizes.iter().map(|&n| merge_work(n)).collect();
    for (n, w) in sizes.iter().zip(&work) {
        eprintln!("[perf] merge({n} entries) work = {w:?}");
    }
    for (&n, w) in sizes.iter().zip(&work) {
        let n = n as u64;
        // The set holds no fork: one resolution, which the gates settle in
        // one pass.
        assert_eq!(w.rounds, 1, "{n}: {w:?}");
        assert!(w.try_apply <= w.rounds * n, "{n}: {w:?}");
        assert!(w.rows_built <= n, "{n}: {w:?}");
        assert!(w.contains <= n, "one anchor query per op: {w:?}");
        // The anchors, one step per dep: the fixture cites at most two.
        assert!(w.anchor_steps <= 2 * n, "{n}: {w:?}");
        assert_eq!(w.frontier_steps, 0, "no removal in the set: {w:?}");
        // A row, when one is built, on this one chain is every op below
        // it, OR'd once from its latest dep, `i / 64` words: Σ i/64 =
        // n²/128 if every row were. Bounded so it grows no further.
        assert!(w.words_ored <= n * n / 128 + n, "{n}: {w:?}");
    }
    for pair in work.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        assert!(at_most_doubles(a.try_apply, b.try_apply), "{a:?} → {b:?}");
        assert!(at_most_doubles(a.contains, b.contains), "{a:?} → {b:?}");
        assert!(at_most_doubles(a.rows_built, b.rows_built), "{a:?} → {b:?}");
        assert!(
            at_most_doubles(a.anchor_steps, b.anchor_steps),
            "{a:?} → {b:?}"
        );
    }
    assert_eq!(work[2], WORK_AT_10K);
}

#[test]
#[ignore = "wall-clock spec; cargo xtask perf on an idle machine"]
fn merge_10k_entries_completes_within_budget() {
    let creator = PseudonymKey([0x42u8; 32]);
    let subkeys = gen_synthetic_entries(ENTRY_COUNT);
    // The op chain is the merge's input, built outside the timed span:
    // the spec times the merge, not the fixture that fakes its ops.
    let ops = ops_in_order(&subkeys);
    let cert = cert(&creator, AdmissionMode::default());
    let started = std::time::Instant::now();
    let _ = merge(&cert, &ops);
    let elapsed = started.elapsed();
    eprintln!(
        "[perf] merge({ENTRY_COUNT} entries) elapsed = {elapsed:?} (spec <{RELEASE_BUDGET_MS}ms)"
    );
    assert!(
        elapsed.as_millis() < RELEASE_BUDGET_MS,
        "merge of {ENTRY_COUNT} governance entries took {elapsed:?}, over the {RELEASE_BUDGET_MS}ms spec"
    );
}

/// Veilid's callee answer window: an `app_call` the inviter answers later
/// than this is dropped (`veilid-core/src/rpc_processor/rpc_app_call.rs:131-138`,
/// `network.rpc.timeout_ms` 5,000 ms, `veilid_config.rs:944`).
const APP_CALL_ANSWER_MS: u128 = 5_000;

fn snapshot_preimage() -> Vec<u8> {
    use rekindle_governance::checkpoint::{state_preimage, state_root_of};
    use rekindle_governance::merge::merge_cut;
    let creator = PseudonymKey([0x42u8; 32]);
    let ops = ops_in_order(&gen_synthetic_entries(ENTRY_COUNT));
    let cert = cert(&creator, AdmissionMode::default());
    let (state, cut) = merge_cut(&cert, &ops);
    let preimage = state_preimage(&state, &cut);
    let _ = state_root_of(&preimage);
    preimage
}

/// Plan E3.2 S7c.8 item (5): the `stateRoot` preimage of the 10,000 entry
/// community is exactly its snapshot's size.
#[test]
fn the_10k_entry_snapshot_preimage_has_its_exact_length() {
    assert_eq!(snapshot_preimage().len(), PREIMAGE_AT_10K);
}

/// Plan E3.2 S7c.8 item (5): the inviter's uncached snapshot answer, the
/// cut's merge (`merge_cut`) and its `stateRoot` preimage, at the 10,000
/// entry community above, inside the `app_call` answer window. Measured
/// debug 265 ms on the Mac (M-series); the kept snapshot (S7c.6) serves
/// every later chunk from its row.
#[test]
#[ignore = "wall-clock spec; cargo xtask perf on an idle machine"]
fn an_uncached_snapshot_of_10k_entries_answers_inside_the_app_call_window() {
    let started = std::time::Instant::now();
    let preimage = snapshot_preimage();
    let elapsed = started.elapsed();
    eprintln!(
        "[perf] merge_cut + stateRoot ({ENTRY_COUNT} entries, {} B snapshot) = {elapsed:?}",
        preimage.len()
    );
    assert!(
        elapsed.as_millis() < APP_CALL_ANSWER_MS,
        "the uncached snapshot took {elapsed:?}, over the {APP_CALL_ANSWER_MS} ms answer window"
    );
}
