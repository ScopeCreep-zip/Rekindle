use super::*;

fn keys(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{i:064x}")).collect()
}

#[test]
fn a_first_inspection_is_cold_and_a_later_one_arrives_after_the_previous() {
    let mut on_screen = OnScreen::default();
    assert_eq!(on_screen.set(keys(2)), keys(2));
    assert_eq!(on_screen.inspected(&keys(1)[0], 1_000), None);
    assert_eq!(on_screen.inspected(&keys(1)[0], 301_000), Some(1_000));
    assert_eq!(on_screen.inspected("not on screen", 5), None);
}

#[test]
fn a_member_kept_keeps_its_inspection_and_one_gone_forgets_it() {
    let mut on_screen = OnScreen::default();
    on_screen.set(keys(2));
    on_screen.inspected(&keys(2)[0], 10);
    on_screen.inspected(&keys(2)[1], 10);
    // Member 1 scrolls away; member 0 stays; member 2 is new.
    let mut next = keys(3);
    next.remove(1);
    assert_eq!(on_screen.set(next), vec![keys(3)[2].clone()]);
    assert_eq!(on_screen.inspected(&keys(2)[0], 20), Some(10));
    // Back on screen: a cold read again.
    on_screen.set(keys(2));
    assert_eq!(on_screen.inspected(&keys(2)[1], 30), None);
    assert_eq!(on_screen.members().count(), 2);
}
