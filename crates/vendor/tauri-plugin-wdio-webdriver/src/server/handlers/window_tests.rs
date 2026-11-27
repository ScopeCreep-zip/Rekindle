use super::*;

fn labels(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

#[test]
fn a_window_the_app_kept_open_comes_first() {
    let handles = kept_window_first(labels(&["main", "splash", "editor"]), "editor");
    assert_eq!(handles, labels(&["editor", "main", "splash"]));
}

#[test]
fn a_closed_window_leaves_the_order_unchanged() {
    let handles = kept_window_first(labels(&["main", "splash"]), "editor");
    assert_eq!(handles, labels(&["main", "splash"]));
}
