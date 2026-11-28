use super::*;

fn test_items() -> Vec<SearchItem> {
    vec![
        SearchItem {
            label: "dev-team".into(),
            detail: "community".into(),
            action: Action::ShowCommunityInfo {
                community: "gov1".into(),
            },
        },
        SearchItem {
            label: "#general".into(),
            detail: "dev-team".into(),
            action: Action::ShowChannel {
                community: "gov1".into(),
                channel: "ch1".into(),
            },
        },
        SearchItem {
            label: "alice".into(),
            detail: "friend".into(),
            action: Action::ShowDmThread {
                peer_key: "pk1".into(),
            },
        },
        SearchItem {
            label: "#random".into(),
            detail: "dev-team".into(),
            action: Action::ShowChannel {
                community: "gov1".into(),
                channel: "ch2".into(),
            },
        },
    ]
}

#[test]
fn open_shows_all_items() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());
    assert!(overlay.visible);
    assert_eq!(overlay.filtered_indices.len(), 4);
}

#[test]
fn typing_filters_results() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());

    // Type "gen" — should match "#general"
    overlay.query = "gen".into();
    overlay.refresh_filter();
    assert_eq!(overlay.filtered_indices.len(), 1);
    assert_eq!(overlay.items[overlay.filtered_indices[0]].label, "#general");
}

#[test]
fn case_insensitive_search() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());

    overlay.query = "ALICE".into();
    overlay.refresh_filter();
    assert_eq!(overlay.filtered_indices.len(), 1);
}

#[test]
fn empty_query_shows_all() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());
    overlay.query.clear();
    overlay.refresh_filter();
    assert_eq!(overlay.filtered_indices.len(), 4);
}

#[test]
fn no_match_shows_empty() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());
    overlay.query = "zzzzz".into();
    overlay.refresh_filter();
    assert!(overlay.filtered_indices.is_empty());
}

#[test]
fn close_clears_state() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());
    overlay.close();
    assert!(!overlay.visible);
    assert!(overlay.items.is_empty());
    assert!(overlay.query.is_empty());
}

#[test]
fn move_selection_clamps() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());
    overlay.move_selection(100); // should clamp to last
    assert_eq!(overlay.list_state.selected(), Some(3));
    overlay.move_selection(-100); // should clamp to first
    assert_eq!(overlay.list_state.selected(), Some(0));
}

#[test]
fn selected_item_index_returns_correct() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());
    // First item should be selected
    assert_eq!(overlay.selected_item_index(), Some(0));
    overlay.move_selection(2);
    assert_eq!(overlay.selected_item_index(), Some(2));
}

#[test]
fn search_by_detail() {
    let mut overlay = SearchOverlay::new();
    overlay.open(SearchMode::QuickSwitch, test_items());
    overlay.query = "friend".into();
    overlay.refresh_filter();
    assert_eq!(overlay.filtered_indices.len(), 1);
    assert_eq!(overlay.items[overlay.filtered_indices[0]].label, "alice");
}
