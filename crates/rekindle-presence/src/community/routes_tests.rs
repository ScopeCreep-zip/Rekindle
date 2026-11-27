use super::*;

fn member(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

#[test]
fn a_newer_seq_replaces_and_an_older_or_equal_one_changes_nothing() {
    let mut cache = MemberRouteCache::default();
    assert!(cache.observe(&member(1), 3, b"A".to_vec()));
    assert!(
        !cache.observe(&member(1), 3, b"B".to_vec()),
        "a repeated seq"
    );
    assert!(
        !cache.observe(&member(1), 2, b"B".to_vec()),
        "a lagging replica"
    );
    assert_eq!(cache.get(&member(1)), Some(&b"A"[..]));
    assert!(cache.observe(&member(1), 4, b"B".to_vec()));
    assert_eq!(cache.get(&member(1)), Some(&b"B"[..]));
    assert!(!cache.observe(&member(2), 9, Vec::new()), "no route");
    assert_eq!(cache.get(&member(2)), None);
}

#[test]
fn forgetting_a_route_takes_the_next_read_at_any_seq() {
    let mut cache = MemberRouteCache::default();
    cache.observe(&member(1), 5, b"A".to_vec());
    assert_eq!(cache.forget(&member(1)), Some(b"A".to_vec()));
    assert_eq!(cache.get(&member(1)), None);
    assert!(
        cache.observe(&member(1), 5, b"A".to_vec()),
        "re-read after forget"
    );
}

#[test]
fn a_dead_blob_is_forgotten_for_every_member_holding_it() {
    let mut cache = MemberRouteCache::default();
    cache.observe(&member(1), 1, b"A".to_vec());
    cache.observe(&member(2), 1, b"A".to_vec());
    cache.observe(&member(3), 1, b"C".to_vec());
    assert_eq!(cache.forget_blobs(|b| b == b"A"), 2);
    assert_eq!(cache.get(&member(1)), None);
    assert_eq!(cache.get(&member(2)), None);
    assert_eq!(cache.get(&member(3)), Some(&b"C"[..]));
    assert!(
        cache.observe(&member(1), 1, b"B".to_vec()),
        "re-read at any seq"
    );
}

#[test]
fn routes_for_a_roster_skips_members_without_a_route_and_retain_drops() {
    let mut cache = MemberRouteCache::default();
    cache.observe(&member(1), 1, b"A".to_vec());
    cache.observe(&member(3), 1, b"C".to_vec());
    let roster = [member(3), member(2), member(1)];
    assert_eq!(
        cache.routes_for(&roster),
        vec![(member(3), b"C".to_vec()), (member(1), b"A".to_vec())]
    );
    let gone = member(3).to_hex();
    cache.retain(|m| m != gone);
    assert_eq!(cache.routes_for(&roster), vec![(member(1), b"A".to_vec())]);
}
