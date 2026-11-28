use super::{scan, slices_a_key};

#[test]
fn a_key_slice_is_found() {
    assert!(slices_a_key("return key.slice(0, 8) + \"...\";"));
    assert!(slices_a_key("{remote.senderPseudonym.slice(0, 8)}"));
    assert!(slices_a_key("peers.map((p) => p.pubkey.slice(0,8))"));
    assert!(slices_a_key("const s = props.peerKey?.slice( 0, 8 );"));
}

#[test]
fn other_slices_are_not() {
    assert!(!slices_a_key("streamId.slice(0, 8)"));
    assert!(!slices_a_key("member.displayName.slice(0, 1)"));
    assert!(!slices_a_key("text.slice(0, 8)"));
    assert!(!slices_a_key("key.slice(-8)"));
}

#[test]
fn scan_names_each_site() {
    let dir = std::env::temp_dir().join(format!("xtask-key-slices-{}", std::process::id()));
    let src = dir.join("src").join("components");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("Tile.tsx"),
        "const a = 1;\nconst label = sender.slice(0, 8);\n",
    )
    .unwrap();
    std::fs::write(src.join("Clean.tsx"), "const t = title.slice(0, 8);\n").unwrap();
    let hits = scan(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        hits,
        vec!["src/components/Tile.tsx:2  const label = sender.slice(0, 8);".to_string()]
    );
}
