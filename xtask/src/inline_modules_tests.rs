use super::{inline_modules_in, Finding};

fn paths(src: &str) -> Vec<String> {
    inline_modules_in(src)
        .expect("parses")
        .into_iter()
        .map(|f| f.path)
        .collect()
}

#[test]
fn flags_an_inline_module_with_its_line() {
    let src = "fn a() {}\n\npub(crate) mod limits {\n    pub const MAX: u8 = 1;\n}\n";
    assert_eq!(
        inline_modules_in(src).expect("parses"),
        vec![Finding {
            line: 3,
            path: "limits".into()
        }]
    );
}

#[test]
fn allows_a_body_of_only_include_items() {
    let src =
        "pub mod foo_capnp {\n    include!(concat!(env!(\"OUT_DIR\"), \"/foo_capnp.rs\"));\n}\n";
    assert!(paths(src).is_empty());
}

#[test]
fn flags_an_include_body_that_also_holds_code() {
    let src = "mod foo {\n    include!(\"x.rs\");\n    fn extra() {}\n}\n";
    assert_eq!(paths(src), ["foo"]);
}

#[test]
fn flags_a_cfg_test_module() {
    let src = "#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn t() {}\n}\n";
    assert_eq!(paths(src), ["tests"]);
}

#[test]
fn flags_nested_inline_modules() {
    let src = "mod outer {\n    mod inner {\n        fn f() {}\n    }\n}\n";
    let found = inline_modules_in(src).expect("parses");
    assert_eq!(
        found,
        vec![
            Finding {
                line: 1,
                path: "outer".into()
            },
            Finding {
                line: 2,
                path: "outer::inner".into()
            },
        ]
    );
}

#[test]
fn flags_a_module_inside_a_function() {
    let src = "fn f() {\n    mod local {\n        pub fn g() {}\n    }\n}\n";
    assert_eq!(paths(src), ["local"]);
}

#[test]
fn ignores_declarations_strings_and_comments() {
    let src = "#[path = \"a_tests.rs\"]\nmod tests;\n// mod x { }\nconst S: &str = \"mod y { }\";\nconst R: &str = r#\"mod z { }\"#;\n/* mod w { } */\n";
    assert!(paths(src).is_empty());
}
