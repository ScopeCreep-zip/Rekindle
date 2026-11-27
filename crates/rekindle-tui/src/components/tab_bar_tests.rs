use super::*;

fn test_tabs() -> Vec<Tab> {
    vec![
        Tab {
            label: "Dashboard".into(),
            id: "dash".into(),
            unread: 0,
        },
        Tab {
            label: "dev-team".into(),
            id: "com1".into(),
            unread: 2,
        },
        Tab {
            label: "gaming".into(),
            id: "com2".into(),
            unread: 0,
        },
        Tab {
            label: "DMs".into(),
            id: "dms".into(),
            unread: 5,
        },
    ]
}

#[test]
fn next_wraps() {
    let mut bar = TabBarState::new(test_tabs());
    assert_eq!(bar.selected, 0);
    bar.next();
    assert_eq!(bar.selected, 1);
    bar.next();
    bar.next();
    bar.next();
    assert_eq!(bar.selected, 0); // wrapped
}

#[test]
fn prev_wraps() {
    let mut bar = TabBarState::new(test_tabs());
    bar.prev();
    assert_eq!(bar.selected, 3); // wrapped to last
}

#[test]
fn select_by_index() {
    let mut bar = TabBarState::new(test_tabs());
    bar.select(2);
    assert_eq!(bar.selected, 2);
    assert_eq!(bar.selected_id(), Some("com2"));
}

#[test]
fn select_out_of_bounds_is_noop() {
    let mut bar = TabBarState::new(test_tabs());
    bar.select(99);
    assert_eq!(bar.selected, 0);
}

#[test]
fn selected_id_returns_correct() {
    let bar = TabBarState::new(test_tabs());
    assert_eq!(bar.selected_id(), Some("dash"));
}

#[test]
fn empty_tabs() {
    let bar = TabBarState::new(Vec::new());
    assert_eq!(bar.selected_id(), None);
}
