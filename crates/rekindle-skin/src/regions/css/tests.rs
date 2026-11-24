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

/// A labelled slot paints its backing once, as one band on its container
/// behind its text row (the measured line, in the content box), never a
/// box per label, and drops it under forced colours.
#[test]
fn a_labelled_slot_paints_one_band_behind_its_text_row() {
    let map: BTreeMap<String, RegionMap> = SLOTS
        .iter()
        .filter(|s| s.css)
        .map(|s| {
            let el = format!(".e-{}", s.id.replace('.', "-"));
            let entry = if s.labelled {
                RegionMap::labelled(&[el.as_str()], &[format!("{el} .label").as_str()])
            } else {
                RegionMap::elements(&[el.as_str()])
            };
            (s.id.to_owned(), entry)
        })
        .collect();
    let css = app_css("t", &map).unwrap();
    assert!(!css.contains(" .label"), "{css}");
    let band = "linear-gradient(var(--skin-region-community-channel-header-band), var(--skin-region-community-channel-header-band))";
    let rules: Vec<&str> = css.split("  }\n").filter(|r| r.contains(band)).collect();
    assert_eq!(rules.len(), 1, "{css}");
    assert!(
        rules[0].contains(".e-community-channel-header::before {"),
        "{}",
        rules[0]
    );
    assert!(
        rules[0].contains(&format!("{band} 0 50% / 100% 21px no-repeat content-box;")),
        "{}",
        rules[0]
    );
    assert!(css.contains(":where(.e-community-channel-header) {\n    position: relative;\n    isolation: isolate;"), "{css}");
    let a11y = app_a11y_css("t", &map).unwrap();
    let forced = &a11y[a11y.find("forced-colors").unwrap()..];
    let none = forced.split("content: none;").next().unwrap();
    assert!(
        none.contains(".e-community-channel-header::before"),
        "{forced}"
    );
}

fn builtin_regions(name: &str) -> Vec<String> {
    let (_, b) = crate::builtin::find(&format!("builtin:{name}")).unwrap();
    let p = crate::package::read(b.bytes).unwrap();
    crate::manifest::parse(&p)
        .unwrap()
        .regions
        .into_keys()
        .collect()
}

/// Symbiosis's main window top row (`MainWindow.xml:12`) fills the titlebar.
#[test]
fn symbiosis_fills_the_titlebar_from_its_top_row() {
    assert!(builtin_regions("symbiosis").contains(&"titlebar".to_owned()));
}

/// Plan E7.7.7 S5: the community window's headers take each family's art:
/// Symbiosis and Classic v3 (XML) and Combat (INI).
#[test]
fn the_community_window_headers_are_skinned_in_every_family() {
    for skin in ["symbiosis", "xfire-classic", "combat"] {
        let r = builtin_regions(skin);
        for slot in ["community.category-header", "community.member-group-header"] {
            assert!(r.contains(&slot.to_owned()), "{skin}: {slot}");
        }
    }
    for skin in ["symbiosis", "xfire-classic"] {
        let r = builtin_regions(skin);
        for slot in ["community.channel-header", "community.sidebar-header"] {
            assert!(r.contains(&slot.to_owned()), "{skin}: {slot}");
        }
    }
}
