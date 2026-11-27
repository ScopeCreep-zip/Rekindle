use super::{findings, SourceModule};

fn scope(module: &[&str], source: &str) -> SourceModule {
    SourceModule {
        module: module.iter().map(|part| (*part).to_owned()).collect(),
        file: format!("{}.rs", module.join("/")).into(),
        items: syn::parse_file(source).expect("fixture parses").items,
    }
}

#[test]
fn flags_debug_derive_and_conditional_derives() {
    let module = scope(
        &["sample"],
        r#"
        use zeroize::{*, ZeroizeOnDrop as Erase};
        #[derive(Clone, Debug, Zeroize)]
        struct Secret([u8; 32]);
        #[cfg_attr(feature = "trace", derive(Debug))]
        #[derive(Erase)]
        enum Other { Key([u8; 32]) }
        #[derive(Debug)]
        struct Public(u32);
    "#,
    );
    let hits = findings(&[module]);
    assert_eq!(
        hits.into_iter().collect::<Vec<_>>(),
        [
            "sample.rs: Other derives Debug",
            "sample.rs: Secret derives Debug",
        ]
    );
}

#[test]
fn flags_formatting_instance_data_for_a_manual_zeroizer_in_another_module() {
    let root = scope(
        &["sample"],
        r#"
        use std::fmt::Debug as Format;
        use self::keys::Secret;
        impl zeroize::ZeroizeOnDrop for Secret {}
        impl Format for Secret {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let Secret(bytes) = self;
                write!(f, "{bytes:?}")
            }
        }
    "#,
    );
    let keys = scope(&["sample", "keys"], "pub struct Secret([u8; 32]);");
    assert_eq!(
        findings(&[root, keys]).into_iter().collect::<Vec<_>>(),
        ["sample.rs: Secret formats instance data",]
    );
}

#[test]
fn accepts_a_redacted_implementation() {
    let module = scope(
        &["sample"],
        r#"
        #[derive(zeroize::ZeroizeOnDrop)]
        struct Secret([u8; 32]);
        impl core::fmt::Debug for Secret {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("Secret(<redacted>)")
            }
        }
    "#,
    );
    assert!(findings(&[module]).is_empty());
}

#[test]
fn follows_imports_reexports_aliases_and_nested_wrappers() {
    let secrets = scope(&["rekindle_secrets"], "pub use self::keys::Secret as Key;");
    let keys = scope(
        &["rekindle_secrets", "keys"],
        r"
        #[derive(zeroize::ZeroizeOnDrop)]
        pub struct Secret([u8; 32]);
    ",
    );
    let wrapper = scope(
        &["sample"],
        r"
        use rekindle_secrets::{self as secrets, Key as Material};
        type Stored = Option<Box<Material>>;
        pub struct Wrapper(Stored);
        #[derive(Debug)]
        enum State { Ready(Wrapper), Waiting }
        #[derive(Debug)]
        struct Marker(core::marker::PhantomData<secrets::Key>);
        #[derive(Debug)]
        struct Secret(u32);
    ",
    );
    assert_eq!(
        findings(&[secrets, keys, wrapper])
            .into_iter()
            .collect::<Vec<_>>(),
        ["sample.rs: State derives Debug",]
    );
}
