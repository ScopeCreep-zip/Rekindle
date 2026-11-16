use super::*;

#[test]
fn properties_keep_slot_and_state_apart() {
    assert_eq!(
        property("scrollbar.button-down", None),
        "--skin-region-scrollbar-button-down"
    );
    assert_eq!(
        property("scrollbar.button-down", Some(RegionState::Down)),
        "--skin-region-scrollbar-button-down--down"
    );
    assert_eq!(
        property("titlebar.btn.close", Some(RegionState::WindowInactive)),
        "--skin-region-titlebar-btn-close--windowinactive"
    );
}

#[test]
fn the_base_alphabet_keeps_css_closed() {
    assert!(base_ok("skin://localhost/0a1b"));
    assert!(base_ok("http://skin.localhost/x_y-z"));
    for bad in ["", "skin://a\"", "a b", "a)", "a;b", "a\\b", "a{"] {
        assert!(!base_ok(bad), "{bad}");
    }
}
