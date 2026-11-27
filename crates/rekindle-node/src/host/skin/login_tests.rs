use super::*;

#[test]
fn the_login_skin_defaults_then_holds_one_copy_at_most() {
    let dir = tempfile::tempdir().unwrap();
    let root = crate::host::fixture::temp_root(&dir);
    assert_eq!(read(&root).unwrap(), builtin::DEFAULT);
    let source = rekindle_skin::validate::run(builtin::PACKS[0].bytes).unwrap();
    let copy = rekindle_skin::login::strip(&source.bytes).unwrap();
    let id = set_copy(&root, &copy).unwrap();
    assert_eq!(read(&root).unwrap(), id);
    let other = Canonical {
        bytes: b"x".to_vec(),
        hash: [7; 32],
    };
    set_copy(&root, &other).unwrap();
    let copies = std::fs::read_dir(root.skins())
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().extension() == Some(EXT.as_ref()))
        .count();
    assert_eq!(copies, 1, "at most one stripped copy");
    set_builtin(&root, builtin::DEFAULT).unwrap();
    assert_eq!(read(&root).unwrap(), builtin::DEFAULT);
    assert_eq!(std::fs::read_dir(root.skins()).unwrap().count(), 1);
}

#[test]
fn a_login_file_naming_nothing_here_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = crate::host::fixture::temp_root(&dir);
    let skins = root.create_skins_dir().unwrap();
    let hash = "ab".repeat(32);
    for bad in [
        format!(r#"{{"skin":"dev:{hash}"}}"#),
        r#"{"skin":"builtin:nope"}"#.to_owned(),
        r#"{"skin":"builtin:symbiosis","x":1}"#.to_owned(),
    ] {
        std::fs::write(skins.join(LOGIN_FILE), &bad).unwrap();
        assert!(read(&root).is_err(), "{bad}");
    }
}
