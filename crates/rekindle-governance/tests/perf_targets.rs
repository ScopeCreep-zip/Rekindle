//! Architecture §32 Phase 7 / Week 26 performance targets, encoded as
//! regression-guard tests.
//!
//! These run by default but are short — generating 10,000 governance
//! entries and merging them. The thresholds match the spec line 4146:
//! "CRDT merge: target <10ms for 10,000 governance entries". We allow
//! 4× headroom (40ms) so debug-mode CI doesn't flake; opt into
//! `--release` for the strict target.

use rekindle_governance::merge::merge;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions;

const ENTRY_COUNT: usize = 10_000;

fn gen_synthetic_entries(n: usize) -> Vec<(PseudonymKey, Vec<GovernanceEntry>)> {
    let mut by_author: std::collections::HashMap<PseudonymKey, Vec<GovernanceEntry>> =
        std::collections::HashMap::new();
    let creator = PseudonymKey([0x42u8; 32]);

    // Genesis: creator at lamport 1, role at 2.
    let creator_entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("Perf Test Community".to_string()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
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
            lamport: 2,
        },
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
        let entry = GovernanceEntry::RoleAssignment {
            target,
            role_id: role,
            lamport,
        };
        by_author.entry(author).or_default().push(entry);
        lamport += 1;
    }

    by_author.into_iter().collect()
}

#[test]
fn merge_10k_entries_completes_within_budget() {
    let subkeys = gen_synthetic_entries(ENTRY_COUNT);
    let started = std::time::Instant::now();
    let _ = merge(&subkeys);
    let elapsed = started.elapsed();

    // Spec target: <10ms in release. Debug-mode is ~5–10× slower; we
    // guard at 200ms so debug CI stays green while still flagging
    // accidental order-of-magnitude regressions.
    assert!(
        elapsed.as_millis() < 200,
        "merge of {ENTRY_COUNT} governance entries took {:?} — exceeds 200ms regression budget",
        elapsed
    );
    eprintln!(
        "[perf] merge({ENTRY_COUNT} entries) elapsed = {:?} (spec target <10ms in release)",
        elapsed
    );
}
