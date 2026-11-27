use super::*;

#[test]
fn only_exhausted_capacity_is_reported() {
    let mut observations = Observations::default();
    observations.values.insert(1, (1, true));
    assert!(!observations.unavailable(2));
    observations.values.insert(2, (1, false));
    assert!(!observations.unavailable(2));
    observations.values.insert(2, (2, true));
    assert!(observations.unavailable(2));
}

#[test]
fn pending_slots_coalesce_and_remain_bounded() {
    let mut batch = Batch::default();
    let mut add = |slots: &[u32]| {
        batch.pending.extend(
            slots
                .iter()
                .copied()
                .filter(|slot| layout::slots(2).contains(slot)),
        )
    };
    add(&[1, 2]);
    add(&[2, 10]);
    assert_eq!(batch.pending, BTreeSet::from([1, 2]));
}
