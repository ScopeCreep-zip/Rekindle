#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a perf test whose measurement IS its output; cargo test --nocapture is where it is read"
)]

//! Architecture §32 Phase 7 / Week 26 performance targets, encoded as
//! regression-guard tests.
//!
//! These run by default but are short — generating 10,000 governance
//! entries and merging them. The spec (communities architecture §32):
//! "CRDT merge: target <10ms for 10,000 governance entries". A build
//! without debug assertions (`--release`, as CI F4 runs it) asserts that
//! target; a debug build asserts a regression guard instead.

use rekindle_governance::fixture::{at, cert, ops_in_order, Stamped};
use rekindle_governance::merge::merge;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry};
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions;

const ENTRY_COUNT: usize = 10_000;

/// The spec target, asserted when built without debug assertions.
const RELEASE_BUDGET_MS: u128 = 10;

/// The debug guard: measured 37–39 ms on the Mac (M-series) after the
/// R4-5 fix, so Linux's ~1.7× slower debug build (~65 ms) stays well
/// under it while a several-fold regression still fails. Release measured
/// 3.2–3.6 ms there.
const DEBUG_BUDGET_MS: u128 = 150;

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
                role_id: RoleId([0u8; 16]),
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
        let role = RoleId([0u8; 16]);
        let entry = at(
            lamport,
            GovernanceEntry::RoleAssignment {
                target,
                role_id: role,
            },
        );
        by_author.entry(author).or_default().push(entry);
        lamport += 1;
    }

    by_author.into_iter().collect()
}

#[test]
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

    let (budget, build) = if cfg!(debug_assertions) {
        (DEBUG_BUDGET_MS, "debug regression guard")
    } else {
        (RELEASE_BUDGET_MS, "spec target")
    };
    eprintln!("[perf] merge({ENTRY_COUNT} entries) elapsed = {elapsed:?} ({build} <{budget}ms)");
    assert!(
        elapsed.as_millis() < budget,
        "merge of {ENTRY_COUNT} governance entries took {elapsed:?}, over the {build} of {budget}ms"
    );
}
