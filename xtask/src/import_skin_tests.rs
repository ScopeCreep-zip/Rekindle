use super::*;

#[test]
fn a_source_correction_is_listed_in_the_notice() {
    assert_eq!(with_corrections("N\n".to_owned(), &[]), "N\n");
    let got = with_corrections("N\n".to_owned(), &["a.xml:5: x -> y".to_owned()]);
    assert!(
        got.ends_with("differs from the author's)\n  a.xml:5: x -> y\n"),
        "{got}"
    );
}

#[test]
fn ids_are_checked() {
    assert!(is_id("carbon-fiber"));
    assert!(!is_id("../x"));
    assert!(!is_id("Carbon"));
    assert!(!is_id(""));
}

#[test]
fn a_pidgin_file_must_be_a_tgz() {
    let e = pidgin_pack(Path::new("theme.zip"), "x").unwrap_err();
    assert!(e.to_string().contains(".tgz/.tar.gz"), "{e}");
}
