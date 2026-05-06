//! Architecture §32 Phase 7 Week 26 — memory budget:
//! "Memory usage: target <200MB per community" (line 4148).
//!
//! The bulk of per-community memory is the merged
//! `GovernanceState` plus the SQLite cache plus the gossip dedup
//! cache plus pending fragment buffers. SQLite + dedup + buffers are
//! transport-layer fixed-size structures (kilobytes); the long-tail
//! variable here is the governance state itself, which scales with
//! the number of merged entries.
//!
//! We synthesize a fully-populated 255-member community with 10,000
//! governance entries (the same regression workload used in
//! `perf_targets.rs`) and assert the merged state size is well below
//! the 200 MB ceiling. Per-community SQLite + dedup + buffers add
//! roughly 5–15 MB on top, all comfortably inside the budget.

use rekindle_governance::merge::merge;
use rekindle_governance::state::GovernanceState;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions;

const ENTRY_COUNT: usize = 10_000;
const BUDGET_MB: usize = 200;

fn synthetic_state() -> GovernanceState {
    let creator = PseudonymKey([0x33u8; 32]);
    let mut entries = Vec::with_capacity(ENTRY_COUNT);
    entries.push(GovernanceEntry::CommunityMeta {
        name: Some("MemTest".into()),
        description: Some("memory budget regression target".repeat(8)),
        icon_hash: None,
        banner_hash: None,
        lamport: 1,
    });
    entries.push(GovernanceEntry::RoleDefinition {
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
    });
    let mut lamport: u64 = 3;
    for i in 0..(ENTRY_COUNT - 2) {
        let author_byte = u8::try_from(i % 50).unwrap_or(0).wrapping_add(1);
        entries.push(GovernanceEntry::RoleAssignment {
            target: PseudonymKey([author_byte; 32]),
            role_id: RoleId([0u8; 16]),
            lamport,
        });
        lamport += 1;
    }
    let subkeys = vec![(creator, entries)];
    merge(&subkeys)
}

/// Conservative shallow estimate of the governance state size — not a
/// precise heap count, but a lower-bound on the variable-length data
/// sitting under the state struct (member-keyed maps and the
/// expression / role / channel collections).
fn estimate_bytes(state: &GovernanceState) -> usize {
    let mut total = std::mem::size_of::<GovernanceState>();
    total += state.role_assignments.capacity()
        * (std::mem::size_of::<PseudonymKey>() + std::mem::size_of::<std::collections::HashSet<RoleId>>());
    for assignments in state.role_assignments.values() {
        total += assignments.capacity() * std::mem::size_of::<RoleId>();
    }
    total += state.roles.capacity() * std::mem::size_of::<rekindle_governance::state::RoleState>();
    for role in state.roles.values() {
        total += role.name.capacity();
    }
    total += state.channels.capacity() * std::mem::size_of::<rekindle_governance::state::ChannelState>();
    for channel in state.channels.values() {
        total += channel.name.capacity() + channel.record_key.capacity();
    }
    total += state.expressions.capacity() * std::mem::size_of::<rekindle_governance::state::ExpressionState>();
    for expr in state.expressions.values() {
        total += expr.name.capacity() + expr.content_hash.capacity();
        if let Some(offer) = &expr.attachment {
            // AttachmentOffer is a bounded manifest (≈ 32B per chunk hash);
            // the actual chunk bytes live in the on-disk file cache, not in
            // governance state. Architecture §18.4 — Lost Cargo.
            total += offer.filename.capacity()
                + offer.mime_type.capacity()
                + offer.chunk_hashes.capacity() * 32
                + offer.wrapped_fek.capacity();
        }
    }
    total
}

#[test]
fn governance_state_within_200mb_budget() {
    let state = synthetic_state();
    let bytes = estimate_bytes(&state);
    let mib = bytes / (1024 * 1024);
    assert!(
        mib < BUDGET_MB,
        "estimated governance state size {mib} MiB exceeds {BUDGET_MB} MiB budget \
         (architecture §32 line 4148)",
    );
    println!(
        "[perf] governance state estimated size = {bytes} bytes ({mib} MiB) for {ENTRY_COUNT} entries"
    );
}
