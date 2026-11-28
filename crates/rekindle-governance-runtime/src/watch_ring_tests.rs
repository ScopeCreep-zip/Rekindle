//! Every member record has exactly `min(k, N − 1)` ring watchers, and one
//! join moves at most k assignments.

use std::collections::BTreeMap;

use super::*;
use rekindle_governance::state::MemberState;

fn state(n: u8) -> GovernanceState {
    let mut state = GovernanceState::default();
    for i in 0..n {
        state.members.insert(
            PseudonymKey([i; 32]),
            MemberState {
                member_record: format!("M{i}"),
                member_record_owner: format!("o{i}"),
                joined: rekindle_governance::op::OpStamp::MIN,
            },
        );
    }
    state
}

fn watchers(state: &GovernanceState) -> BTreeMap<String, Vec<PseudonymKey>> {
    let mut out: BTreeMap<String, Vec<PseudonymKey>> = BTreeMap::new();
    let mut members: Vec<_> = state.members.keys().cloned().collect();
    members.sort_unstable_by_key(|p| p.0);
    for p in members {
        for record in ring_successors("A", state, &p, RING_K) {
            out.entry(record).or_default().push(p.clone());
        }
    }
    out
}

#[test]
fn every_record_has_min_k_n_minus_1_watchers() {
    for n in [1u8, 2, 3, 5, 40] {
        let state = state(n);
        let w = watchers(&state);
        let want = RING_K.min(usize::from(n) - 1);
        for m in state.members.values() {
            let got = w.get(&m.member_record).map_or(0, Vec::len);
            assert_eq!(got, want, "N = {n}: {}", m.member_record);
        }
    }
}

#[test]
fn a_join_moves_at_most_k_assignments() {
    let before = state(40);
    let after = state(41);
    let assigned = |s: &GovernanceState| -> Vec<(PseudonymKey, String)> {
        let mut v: Vec<_> = before
            .members
            .keys()
            .flat_map(|p| {
                ring_successors("A", s, p, RING_K)
                    .into_iter()
                    .map(move |r| (p.clone(), r))
            })
            .collect();
        v.sort_unstable_by(|a, b| a.0 .0.cmp(&b.0 .0).then_with(|| a.1.cmp(&b.1)));
        v
    };
    let (old, new) = (assigned(&before), assigned(&after));
    let moved = new.iter().filter(|a| !old.contains(a)).count();
    assert!(moved <= RING_K, "{moved} assignments moved");
}

#[test]
fn a_member_not_admitted_watches_nobody() {
    assert!(ring_successors("A", &state(5), &PseudonymKey([99; 32]), RING_K).is_empty());
}

/// The creator is in its own ring: the merge binds it to the cert's member
/// record (the genesis ops hold no `MemberJoined`), so with 5 members it
/// watches 4 successors and its `M` has 4 watchers.
#[test]
fn the_creator_is_in_its_own_ring() {
    use rekindle_governance::fixture::{at, merge_entries, meta};
    let creator = PseudonymKey([200; 32]);
    let mut merged = merge_entries(&creator, &[(creator.clone(), vec![at(1, meta("c"))])]);
    let own = merged
        .members
        .get(&creator)
        .map(|m| m.member_record.clone());
    assert_eq!(own.as_deref(), Some("VLD0:creator-record"));
    for (p, m) in state(4).members {
        merged.members.insert(p, m);
    }
    assert_eq!(
        ring_successors("A", &merged, &creator, RING_K).len(),
        RING_K
    );
    let w = watchers(&merged);
    assert_eq!(w.get("VLD0:creator-record").map_or(0, Vec::len), RING_K);
}
