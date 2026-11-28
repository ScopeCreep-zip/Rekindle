use super::*;
use crate::import::tree::parse;

const OUTSETS: Insets = Insets {
    left: 3,
    top: 2,
    right: 7,
    bottom: 9,
};

#[test]
fn authored_root_axes_use_tree_outsets_only_once() {
    let root = parse(
        "main.xml",
        br#"<Component DefaultWidth="333" MaxHeight="600"/>"#,
    )
    .unwrap();
    let hints = root_size_hints(Some(&root), OUTSETS).unwrap();
    assert_eq!(hints.default_width, Some(343));
    assert_eq!(hints.default_height, None);
    assert_eq!(hints.max_width, None);
    assert_eq!(hints.max_height, Some(611));
    assert!(root_size_hints(None, OUTSETS).unwrap().is_empty());
}

#[test]
fn malformed_zero_and_overflowing_root_sizes_are_refused() {
    for raw in ["0", "large", "65535"] {
        let xml = format!(r#"<Component MaxWidth="{raw}"/>"#);
        let root = parse("main.xml", xml.as_bytes()).unwrap();
        assert!(root_size_hints(Some(&root), OUTSETS).is_err(), "{raw}");
    }
}

#[test]
fn ini_root_carries_default_height_but_a_child_cannot_supply_root_width() {
    use crate::import::xfire_ini::{ini, title};

    let raw = b"[MainWindow]\nDefaultHeight=555\nWindow1.Name=Child\n\
                [Child]\nDefaultWidth=600\n";
    let ini = ini::parse("fixture.ini", raw).unwrap();
    let (_, components) = title::tree(&ini, "MainWindow").unwrap();
    let main = components
        .children
        .iter()
        .find(|node| node.attr_ci("Name") == Some("MainWindow"))
        .unwrap();
    let hints = root_size_hints(Some(main), OUTSETS).unwrap();
    assert_eq!(hints.default_height, Some(566));
    assert_eq!(hints.default_width, None);
}
