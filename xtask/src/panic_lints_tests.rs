use super::{lowered_in, root_findings};

const ATTR: &str = "cfg_attr(not(test), deny(clippy::indexing_slicing, \
    clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::expect_used, \
    clippy::panic, clippy::string_slice))";

#[test]
fn a_crate_attribute_holds() {
    let src = format!("#![{ATTR}]\npub mod a;\n");
    assert!(root_findings(&src).unwrap().is_empty());
}

#[test]
fn a_module_macro_beside_generated_modules_holds() {
    let src = format!(
        "macro_rules! m {{ ($x:ident) => {{ #[{ATTR}] pub mod $x; }}; }}\n\
         macro_rules! g {{ ($x:ident) => {{ pub mod $x {{ include!(concat!(env!(\"OUT_DIR\"), \"/x.rs\")); }} }}; }}\n\
         m!(a);\nm!(b);\ng!(x_capnp);\n"
    );
    assert!(root_findings(&src).unwrap().is_empty());
}

#[test]
fn a_bare_module_fails() {
    let src = format!(
        "macro_rules! m {{ ($x:ident) => {{ #[{ATTR}] pub mod $x; }}; }}\nm!(a);\npub mod b;\n"
    );
    let f = root_findings(&src).unwrap();
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].contains("`mod b`"));
}

#[test]
fn a_missing_lint_fails() {
    let src = "#![cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::unwrap_used))]\npub mod a;\n";
    assert!(!root_findings(src).unwrap().is_empty());
}

#[test]
fn a_lint_denied_in_tests_only_fails() {
    let src = "#![deny(clippy::indexing_slicing)]\npub mod a;\n";
    assert!(!root_findings(src).unwrap().is_empty());
}

#[test]
fn an_unknown_root_macro_fails() {
    let src = format!(
        "macro_rules! m {{ ($x:ident) => {{ #[{ATTR}] pub mod $x; }}; }}\nm!(a);\nother!(b);\n"
    );
    assert_eq!(root_findings(&src).unwrap().len(), 1);
}

#[test]
fn lowering_a_lint_anywhere_fails() {
    assert_eq!(
        lowered_in("#[allow(clippy::unwrap_used, reason = \"x\")]\nfn f() {}\n")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        lowered_in("fn f() { #[expect(clippy::indexing_slicing)] let _ = 1; }\n")
            .unwrap()
            .len(),
        1
    );
    assert!(
        lowered_in("#[allow(clippy::all, clippy::pedantic)]\nmod g {}\n")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        lowered_in("macro_rules! m { () => { #[allow(clippy::panic)] mod x; }; }\n")
            .unwrap()
            .len(),
        1
    );
}
