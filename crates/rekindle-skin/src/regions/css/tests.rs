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

#[test]
fn a_status_icon_is_served_with_its_paint_switch() {
    let (_, b) = crate::builtin::find(crate::builtin::DEFAULT).unwrap();
    let p = crate::package::read(b.bytes).unwrap();
    let mut m = crate::manifest::parse(&p).unwrap();
    let mut images: BTreeMap<String, Vec<u8>> =
        p.files().map(|(n, b)| (n.to_owned(), b.to_vec())).collect();
    let png = vec![0x89, b'P', b'N', b'G'];
    images.insert("icons/status.away.png".to_owned(), png.clone());
    m.icons
        .insert("status.away".to_owned(), "icons/status.away.png".to_owned());
    let css = skin_css(&m, &images, "skin://x").unwrap();
    let h = blake3::hash(&png).to_hex();
    assert!(
        css.contains(&format!(
            "    --skin-icon-status-away: url(\"skin://x/{h}.png\");\n"
        )),
        "{css}"
    );
    assert!(css.contains("    --skin-icon-status-away-paint: transparent;\n"));
    assert!(!css.contains("--skin-icon-status-online"));
}
