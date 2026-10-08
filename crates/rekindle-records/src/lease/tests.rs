//! The lease table's invariants: the refcount Veilid does not keep.

use proptest::prelude::*;

use super::*;

fn set(subkeys: &[u32]) -> SubkeySet {
    subkeys.iter().copied().collect()
}

#[test]
fn the_first_borrow_opens_and_the_last_release_closes() {
    let mut table = LeaseTable::<&str>::default();
    let (a, plan) = table.acquire("k", None);
    assert_eq!(plan, OpenPlan::Open { writer: None });
    let (b, plan) = table.acquire("k", None);
    assert_eq!(plan, OpenPlan::AlreadyOpen);
    assert_eq!(table.release(a), None);
    assert!(table.is_held("k"));
    assert_eq!(table.release(b), Some("k".to_string()));
    assert!(!table.is_held("k"));
    assert_eq!(table.release(b), None, "a second release is a no-op");
}

#[test]
fn releasing_a_watcher_shrinks_the_watch_to_the_remaining_borrowers() {
    let mut table = LeaseTable::<&str>::default();
    let (a, _) = table.acquire("k", None);
    let (b, _) = table.acquire("k", None);
    let (c, _) = table.acquire("k", None);
    table.set_watch(a, set(&[1, 2]));
    table.set_watch(b, set(&[3]));
    assert_eq!(
        table.release_plan(a),
        ReleasePlan::Rewatch("k".into(), WatchPlan::Watch(set(&[3])))
    );
    assert_eq!(
        table.release_plan(c),
        ReleasePlan::Kept,
        "c watched nothing"
    );
    let (d, _) = table.acquire("k", None);
    assert_eq!(
        table.release_plan(b),
        ReleasePlan::Rewatch("k".into(), WatchPlan::Cancel),
        "the last watcher left; d still holds the record"
    );
    assert_eq!(table.release_plan(d), ReleasePlan::Closed("k".into()));
}

#[test]
fn a_writer_upgrades_once_and_never_downgrades_or_changes() {
    let mut table = LeaseTable::default();
    let (_, plan) = table.acquire("k", None);
    assert_eq!(plan, OpenPlan::Open { writer: None });
    let (w1, plan) = table.acquire("k", Some("owner"));
    assert_eq!(plan, OpenPlan::Upgrade { writer: "owner" });
    assert_eq!(table.acquire("k", None).1, OpenPlan::AlreadyOpen);
    assert_eq!(table.acquire("k", Some("other")).1, OpenPlan::AlreadyOpen);
    assert_eq!(table.writer("k"), Some(&"owner"));
    table.release(w1);
    assert_eq!(table.writer("k"), Some(&"owner"), "sticky while held");
}

#[test]
fn an_aborted_upgrade_leaves_the_record_without_that_writer() {
    let mut table = LeaseTable::default();
    let (_, _) = table.acquire("k", None);
    let (w, plan) = table.acquire("k", Some("owner"));
    assert_eq!(table.abort(w, &plan), None);
    assert_eq!(table.writer("k"), None);
    let (only, plan) = table.acquire("j", Some("owner"));
    assert_eq!(table.abort(only, &plan), Some("j".to_string()));
    assert!(!table.is_held("j"));
}

#[test]
fn the_watch_is_the_union_and_the_last_watcher_cancels_it() {
    let mut table = LeaseTable::<()>::default();
    let (a, _) = table.acquire("k", None);
    let (b, _) = table.acquire("k", None);
    assert_eq!(
        table.set_watch(a, set(&[0, 1])),
        WatchPlan::Watch(set(&[0, 1]))
    );
    assert_eq!(
        table.set_watch(b, set(&[1, 2])),
        WatchPlan::Watch(set(&[0, 1, 2]))
    );
    assert_eq!(
        table.set_watch(b, set(&[1])),
        WatchPlan::Watch(set(&[0, 1]))
    );
    assert_eq!(table.set_watch(a, set(&[0, 1])), WatchPlan::Unchanged);
    assert_eq!(table.set_watch(b, SubkeySet::new()), WatchPlan::Unchanged);
    assert_eq!(table.set_watch(a, SubkeySet::new()), WatchPlan::Cancel);
    assert!(table.watched("k").is_empty());
}

#[test]
fn subkey_caps_match_the_veilid_formula() {
    let cap = |n| max_subkey_bytes(SchemaShape { subkey_count: n });
    assert_eq!(cap(255), 4112, "SMPL(0, 255×1)");
    assert_eq!(cap(1), 32_768, "DFLT(1)");
    assert_eq!(cap(8), 32_768, "DFLT(8)");
    assert_eq!(cap(101), 10_381, "DFLT(101)");
    assert_eq!(cap(256), 4096, "DFLT(256)");
    assert_eq!(cap(0), 0);
}

#[test]
fn community_leases_list_every_lease() {
    let leases = CommunityLeases {
        governance: Some(LeaseId(1)),
        registry: Some(LeaseId(2)),
        channels: HashMap::from([(ChannelId([0; 16]), LeaseId(3))]),
        segments: vec![LeaseId(4)],
        overflow: vec![LeaseId(5)],
    };
    assert_eq!(leases.all().count(), 5);
}

#[derive(Debug, Clone)]
enum Op {
    Acquire { key: u8, writer: Option<u8> },
    Release(usize),
    Watch(usize, Vec<u32>),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0u8..3, proptest::option::of(0u8..3))
            .prop_map(|(key, writer)| Op::Acquire { key, writer }),
        (0usize..32).prop_map(Op::Release),
        (0usize..32, proptest::collection::vec(0u32..8, 0..4)).prop_map(|(i, s)| Op::Watch(i, s)),
    ]
}

proptest! {
    /// Under any interleaving: a record is held exactly while it has a
    /// borrower; its writer, once set, never changes; its watch is the union
    /// of its borrowers' subkeys; and the plans returned describe exactly
    /// that (an open per first borrow, a close per last release).
    #[test]
    fn invariants_hold_under_any_interleaving(ops in proptest::collection::vec(op(), 0..64)) {
        let mut table = LeaseTable::<u8>::default();
        let mut live: Vec<(LeaseId, String, SubkeySet)> = Vec::new();
        let mut first_writer: BTreeMap<String, u8> = BTreeMap::new();
        for op in ops {
            match op {
                Op::Acquire { key, writer } => {
                    let key = format!("k{key}");
                    let was_held = live.iter().any(|(_, k, _)| *k == key);
                    let (id, plan) = table.acquire(&key, writer);
                    match plan {
                        OpenPlan::Open { .. } => prop_assert!(!was_held),
                        OpenPlan::AlreadyOpen | OpenPlan::Upgrade { .. } => prop_assert!(was_held),
                    }
                    if let Some(w) = writer {
                        first_writer.entry(key.clone()).or_insert(w);
                    }
                    live.push((id, key, SubkeySet::new()));
                }
                Op::Release(i) if !live.is_empty() => {
                    let (id, key, _) = live.remove(i % live.len());
                    let still_held = live.iter().any(|(_, k, _)| *k == key);
                    let closed = table.release(id);
                    prop_assert_eq!(closed.is_some(), !still_held);
                    if !still_held {
                        first_writer.remove(&key);
                    }
                }
                Op::Watch(i, subkeys) if !live.is_empty() => {
                    let i = i % live.len();
                    let s: SubkeySet = subkeys.into_iter().collect();
                    live[i].2 = s.clone();
                    table.set_watch(live[i].0, s);
                }
                Op::Release(_) | Op::Watch(..) => {}
            }
            for key in ["k0", "k1", "k2"] {
                let holders: Vec<_> = live.iter().filter(|(_, k, _)| k == key).collect();
                prop_assert_eq!(table.is_held(key), !holders.is_empty());
                let union: SubkeySet = holders.iter().flat_map(|(_, _, s)| s.iter().copied()).collect();
                prop_assert_eq!(table.watched(key), union);
                prop_assert_eq!(table.writer(key), first_writer.get(key));
            }
        }
    }
}

#[test]
fn community_merge_keeps_one_lease_per_record() {
    let keys: HashMap<u64, &str> =
        HashMap::from([(1, "gov"), (2, "reg"), (3, "gov"), (4, "page"), (5, "page")]);
    let key_of = |l: LeaseId| keys.get(&l.0).map(|k| (*k).to_string());
    let mut held = CommunityLeases::default();

    let first = held.merge(
        CommunityLeases {
            governance: Some(LeaseId(1)),
            registry: Some(LeaseId(2)),
            overflow: vec![LeaseId(4), LeaseId(5)],
            ..Default::default()
        },
        key_of,
    );
    // Two leases on one page: the second is surplus.
    assert_eq!(first.surplus, vec![LeaseId(5)]);
    assert_eq!(
        first.added,
        vec![
            (HeldKind::Governance, "gov".to_string()),
            (HeldKind::Registry, "reg".to_string()),
            (HeldKind::Overflow, "page".to_string()),
        ]
    );

    // A repeat hand-over of the governance record, and a lease the pool no
    // longer knows (99), are both surplus; nothing is added.
    let second = held.merge(
        CommunityLeases {
            governance: Some(LeaseId(3)),
            segments: vec![LeaseId(99)],
            ..Default::default()
        },
        key_of,
    );
    assert_eq!(second.surplus, vec![LeaseId(3), LeaseId(99)]);
    assert!(second.added.is_empty());
    assert_eq!(held.governance, Some(LeaseId(1)));
    assert_eq!(held.all().count(), 3);
}
