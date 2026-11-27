use super::*;

#[test]
fn all_keys_includes_the_overflow_chain() {
    let mut r = CommunityRecords::new("gov".into(), "reg".into());
    r.add_segment("seg1".into());
    r.add_segment("seg1".into()); // repeat ignored
    r.add_overflow("of1".into());
    assert_eq!(r.all_keys(), vec!["gov", "reg", "seg1", "of1"]);
    assert_eq!(r.record_count(), 4);
}
