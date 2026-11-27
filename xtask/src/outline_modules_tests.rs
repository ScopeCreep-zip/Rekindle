use super::outline_source;

#[test]
fn moves_a_cfg_test_module_beside_its_parent() {
    let src = "pub fn double(x: u8) -> u8 {\n    x * 2\n}\n\n/// Unit tests.\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn doubles() {\n        assert_eq!(double(2), 4);\n    }\n}\n";
    let out = outline_source(src, "math").expect("outlines");
    assert_eq!(
        out.parent,
        "pub fn double(x: u8) -> u8 {\n    x * 2\n}\n\n/// Unit tests.\n#[cfg(test)]\n#[path = \"math_tests.rs\"]\nmod tests;\n"
    );
    assert_eq!(
        out.files,
        vec![(
            "math_tests.rs".to_owned(),
            "use super::*;\n\n#[test]\nfn doubles() {\n    assert_eq!(double(2), 4);\n}\n"
                .to_owned()
        )]
    );
    // The parent still declares the same module under the same attributes,
    // and the moved body still reaches its parent through `super`.
    let parent = syn::parse_file(&out.parent).expect("parent parses");
    let syn::Item::Mod(m) = &parent.items[1] else {
        panic!("second item is the module");
    };
    assert!(m.content.is_none());
    assert!(m.attrs.iter().any(|a| a.path().is_ident("doc")));
    assert!(m.attrs.iter().any(|a| a.path().is_ident("cfg")));
    assert!(out.files[0].1.starts_with("use super::*;"));
}

#[test]
fn keeps_visibility_and_moves_nested_modules_again() {
    let src = "pub(crate) mod outer {\n    pub fn a() {}\n\n    mod inner {\n        fn b() {}\n    }\n}\n";
    let out = outline_source(src, "lib").expect("outlines");
    assert_eq!(
        out.parent,
        "#[path = \"lib_outer.rs\"]\npub(crate) mod outer;\n"
    );
    assert_eq!(
        out.files,
        vec![
            (
                "lib_outer.rs".to_owned(),
                "pub fn a() {}\n\n#[path = \"lib_outer_inner.rs\"]\nmod inner;\n".to_owned()
            ),
            ("lib_outer_inner.rs".to_owned(), "fn b() {}\n".to_owned()),
        ]
    );
}

#[test]
fn leaves_multiline_string_contents_untouched() {
    let src = "mod tests {\n    const S: &str = \"a\n  b\";\n    const R: &str = r#\"\n        x\"#;\n}\n";
    let out = outline_source(src, "m").expect("outlines");
    assert_eq!(
        out.files[0].1,
        "const S: &str = \"a\n  b\";\nconst R: &str = r#\"\n        x\"#;\n"
    );
}

#[test]
fn leaves_include_only_modules_alone() {
    let src =
        "pub mod foo_capnp {\n    include!(concat!(env!(\"OUT_DIR\"), \"/foo_capnp.rs\"));\n}\n";
    let out = outline_source(src, "lib").expect("outlines");
    assert_eq!(out.parent, src);
    assert!(out.files.is_empty());
}

#[test]
fn refuses_a_body_with_a_file_backed_module() {
    let err = outline_source("mod outer {\n    mod child;\n}\n", "a").expect_err("refuses");
    assert!(err.to_string().contains("file-backed"), "{err}");
}

#[test]
fn refuses_a_module_inside_a_function() {
    let err = outline_source("fn f() {\n    mod local {}\n}\n", "a").expect_err("refuses");
    assert!(err.to_string().contains("inside a function"), "{err}");
}
