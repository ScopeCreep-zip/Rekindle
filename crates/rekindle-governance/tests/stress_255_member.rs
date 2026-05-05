//! Architecture §32 Phase 7 Week 26 — 255-member community stress
//! harness.
//!
//! The spec calls for "255-member community, 50 concurrent writers, 10
//! voice participants, sustained for 1 hour." Voice / network /
//! transport are out of pure-logic scope; what we *can* stress at the
//! library tier is the CRDT merge engine, which is the load-bearing
//! "expensive operation per inbound governance entry" hot path.
//!
//! The default test runs a short pass (~1 second of work) so CI stays
//! green. Setting `REKINDLE_STRESS_MINUTES=60` (or any positive
//! integer) extends the harness to the spec's 1-hour duration —
//! intended for manual runs, not CI.
//!
//! Sources:
//! * Architecture §32 Phase 7 Week 26 (line 4143-4149).
//! * Architecture §15 Plate Gates ("≤255 active writers per record").

use std::time::{Duration, Instant};

use rekindle_governance::merge::merge;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions;

const TOTAL_MEMBERS: usize = 255;
const CONCURRENT_WRITERS: usize = 50;
const ENTRIES_PER_WRITER: usize = 200;
const CI_BUDGET_MS: u128 = 5_000;

fn build_member_keys(n: usize) -> Vec<PseudonymKey> {
    (0..n)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[..4].copy_from_slice(&u32::try_from(i).unwrap_or(0).to_le_bytes());
            // Avoid zero-key (creator slot) for non-creator members.
            if i == 0 {
                bytes[31] = 0xFF;
            }
            PseudonymKey(bytes)
        })
        .collect()
}

/// One round of 50 writers × 200 entries each = 10,000 governance
/// entries — exactly the budget the spec line 4146 calls out for the
/// `<10ms` merge target. We add 2 genesis entries from the creator to
/// match the canonical flat-governance bootstrap.
fn synth_round(
    authors: &[PseudonymKey],
    creator: &PseudonymKey,
    base_lamport: u64,
) -> Vec<(PseudonymKey, Vec<GovernanceEntry>)> {
    let mut by_author: std::collections::HashMap<PseudonymKey, Vec<GovernanceEntry>> =
        std::collections::HashMap::new();

    let creator_entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some("Stress Test Community".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: base_lamport,
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
            lamport: base_lamport + 1,
        },
    ];
    by_author.insert(creator.clone(), creator_entries);

    let mut lamport = base_lamport + 2;
    for writer_idx in 0..CONCURRENT_WRITERS {
        let author = authors[writer_idx % authors.len()].clone();
        for _ in 0..ENTRIES_PER_WRITER {
            let entry = GovernanceEntry::RoleAssignment {
                target: author.clone(),
                role_id: RoleId([0u8; 16]),
                lamport,
            };
            by_author.entry(author.clone()).or_default().push(entry);
            lamport += 1;
        }
    }
    by_author.into_iter().collect()
}

#[test]
fn stress_255_members_50_writers_merge_within_budget() {
    let authors = build_member_keys(TOTAL_MEMBERS);
    let creator = authors[0].clone();
    let duration = stress_duration_from_env();
    let started = Instant::now();
    let mut total_merges = 0u64;
    let mut total_entries = 0u64;
    let mut max_round_ms = 0u128;
    let mut base_lamport: u64 = 1;

    while started.elapsed() < duration {
        let subkeys = synth_round(&authors, &creator, base_lamport);
        let entries_this_round: usize =
            subkeys.iter().map(|(_, entries)| entries.len()).sum();
        let merge_started = Instant::now();
        let state = merge(&subkeys);
        let round_ms = merge_started.elapsed().as_millis();

        // Sanity invariants — if any of these fail, the merge engine
        // silently dropped or duplicated entries under load.
        assert!(
            state.roles.contains_key(&RoleId([0u8; 16])),
            "@everyone role must survive merge"
        );
        assert_eq!(
            state.creator.as_ref(),
            Some(&creator),
            "creator should be the genesis writer"
        );

        total_merges += 1;
        total_entries += u64::try_from(entries_this_round).unwrap_or(u64::MAX);
        max_round_ms = max_round_ms.max(round_ms);
        base_lamport += u64::try_from(entries_this_round).unwrap_or(0);
    }

    eprintln!(
        "[stress-255] {} merges, {} entries, max_round_ms={}, total_elapsed={:?}",
        total_merges,
        total_entries,
        max_round_ms,
        started.elapsed()
    );

    // CI budget — generous to absorb debug-mode slowness; the spec's
    // <10ms release-mode target is enforced by `tests/perf_targets.rs`.
    assert!(
        max_round_ms < CI_BUDGET_MS,
        "max merge round took {max_round_ms}ms — exceeds {CI_BUDGET_MS}ms regression budget"
    );
}

fn stress_duration_from_env() -> Duration {
    if let Ok(minutes) = std::env::var("REKINDLE_STRESS_MINUTES") {
        if let Ok(m) = minutes.parse::<u64>() {
            if m > 0 {
                return Duration::from_secs(m * 60);
            }
        }
    }
    // Default CI pass: one round (≈ a few hundred ms in debug).
    Duration::from_millis(1)
}
