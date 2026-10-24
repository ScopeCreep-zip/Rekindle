use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{scan, READER_FILE};

#[test]
fn a_reader_outside_the_codec_is_refused() {
    let src = "fn f(b: &[u8]) { capnp::serialize_packed::read_message(b, \
               capnp::message::ReaderOptions::new()); }";
    assert_eq!(
        scan("crates/x/src/lib.rs", src).len(),
        2,
        "the reader and its options"
    );
    let used = "use capnp::serialize::read_message_from_flat_slice;";
    assert_eq!(scan("crates/x/src/lib.rs", used).len(), 1);
    let grouped = "use capnp::{serialize_packed::{read_message as r}};";
    assert_eq!(scan("crates/x/src/lib.rs", grouped).len(), 1);
    let codec_fn = "use super::read::{read_message_edited};";
    assert!(scan("crates/x/src/lib.rs", codec_fn).is_empty());
}

/// The three spellings that once passed: an aliased module, a glob, and
/// a reader built from segments.
#[test]
fn a_reader_behind_another_name_is_refused() {
    let lib = "crates/x/src/lib.rs";
    let alias = "use capnp::serialize_packed as sp; \
                 fn f(b: &[u8]) { sp::read_message(b, Default::default()); }";
    assert!(!scan(lib, alias).is_empty());
    let glob = "use capnp::serialize::*; \
                fn f(b: &[u8]) { read_message(b, Default::default()); }";
    assert!(!scan(lib, glob).is_empty());
    let segments = "fn f(b: &[u8]) { capnp::message::Reader::new(\
                    capnp::serialize::BufferSegments::new(b, Default::default()).unwrap(), \
                    Default::default()); }";
    assert!(!scan(lib, segments).is_empty());
    let options = "use capnp::message::ReaderOptions as O; fn f() { O::new(); }";
    assert!(!scan(lib, options).is_empty());
    let reader = "use capnp::message::{Reader as R}; fn f() {}";
    assert!(!scan(lib, reader).is_empty());
    let crate_alias = "use capnp as c; fn f() {}";
    assert!(!scan(lib, crate_alias).is_empty());
    let extern_alias = "extern crate capnp as c;";
    assert!(!scan(lib, extern_alias).is_empty());
    let module_alias = "use capnp::{message as m};";
    assert!(!scan(lib, module_alias).is_empty());
    let plain = "use capnp::message; use crate::chat::message::*; \
                 fn f(b: &message::Builder<message::HeapAllocator>) {}";
    assert!(scan(lib, plain).is_empty());
}

#[test]
fn the_codec_reader_needs_a_limit() {
    let unbounded = "fn unpack_bounded(b: &[u8]) { capnp::serialize_packed::read_message(b, \
                     capnp::message::ReaderOptions::new()); }";
    assert_eq!(scan(READER_FILE, unbounded).len(), 1);
    let bounded = "fn unpack_bounded(b: &[u8], bound: Bound) { \
                   let mut o = capnp::message::ReaderOptions::new(); \
                   o.traversal_limit_in_words(Some(bound.max_words)); \
                   capnp::serialize_packed::read_message(b, o); }";
    assert!(scan(READER_FILE, bounded).is_empty());
    for limit in [
        "Some(usize::MAX)",
        "Option::None",
        "Some(words)",
        "Some(other.max_words)",
        "limit",
    ] {
        let src = format!(
            "fn unpack_bounded(b: &[u8], bound: Bound) {{ \
             let mut o = capnp::message::ReaderOptions::new(); \
             o.traversal_limit_in_words({limit}); \
             capnp::serialize_packed::read_message(b, o); }}"
        );
        assert!(!scan(READER_FILE, &src).is_empty(), "{limit}");
    }
    let elsewhere = "fn other(b: &[u8], bound: Bound) { \
                     let mut o = capnp::message::ReaderOptions::new(); \
                     o.traversal_limit_in_words(Some(bound.max_words)); \
                     capnp::serialize_packed::read_message(b, o); }";
    assert!(!scan(READER_FILE, elsewhere).is_empty());
    let none = "fn unpack_bounded() { let mut o = capnp::message::ReaderOptions::new(); \
                o.traversal_limit_in_words(None); \
                capnp::serialize_packed::read_message(b, o); }";
    assert_eq!(scan(READER_FILE, none).len(), 2);
}

#[test]
fn every_call_names_a_bound_const() {
    let lib = "crates/rekindle-codec/src/x.rs";
    let named = "fn f(b: &[u8]) { unpack_bounded(b, bounds::SIGNED_ENVELOPE); }";
    assert!(scan(lib, named).is_empty());
    for bound in [
        "bounds::NOT_A_BOUND",
        "SIGNED_ENVELOPE",
        "Bound::packed(9)",
        "Bound { max_bytes: 1, max_words: usize::MAX }",
        "b",
    ] {
        let src = format!("fn f(b: &[u8]) {{ unpack_bounded(b, {bound}); }}");
        assert!(!scan(lib, &src).is_empty(), "{bound}");
    }
    let max = "pub const X: Bound = Bound { max_bytes: 1, max_words: usize::MAX };";
    assert!(!scan(super::BOUNDS_FILE, max).is_empty());
    let names =
        super::bound_names("pub const A: Bound = Bound::packed(1); pub const B: usize = 2;");
    assert_eq!(names.into_iter().collect::<Vec<_>>(), ["A"]);
}

#[test]
fn only_the_codec_may_depend_on_capnp() {
    let allowed: BTreeSet<String> = ["rekindle-codec", "capnpc", "veilid-core"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert!(super::capnp_dependent_hits(&allowed).is_empty());
    let mut with_protocol = allowed;
    with_protocol.insert("rekindle-protocol".to_owned());
    assert_eq!(super::capnp_dependent_hits(&with_protocol).len(), 1);
}

#[test]
fn a_file_that_does_not_parse_is_refused() {
    assert_eq!(scan("crates/x/src/lib.rs", "fn (").len(), 1);
}

/// Test files are known from the module tree, not their names.
#[test]
fn test_files_come_from_cfg_test_declarations() {
    let lib = (
        PathBuf::from("/c/src/lib.rs"),
        "mod codec; #[cfg(test)] mod tests; #[cfg(test)] #[path = \"x_t.rs\"] mod x;".to_owned(),
    );
    let codec = (
        PathBuf::from("/c/src/codec.rs"),
        "mod tests; mod inner { #[cfg(test)] mod deep; }".to_owned(),
    );
    let roots = [PathBuf::from("/c/src/lib.rs")].into_iter().collect();
    let t = super::test_modules(&[lib, codec], &roots);
    assert!(t.contains(Path::new("/c/src/tests.rs")));
    assert!(t.contains(Path::new("/c/src/tests/helpers.rs")));
    assert!(t.contains(Path::new("/c/src/x_t.rs")));
    assert!(t.contains(Path::new("/c/src/codec/inner/deep.rs")));
    // A production module that happens to be named `tests`.
    assert!(!t.contains(Path::new("/c/src/codec/tests.rs")));
    assert!(!t.contains(Path::new("/c/src/codec.rs")));
}

#[test]
fn test_code_is_not_scanned() {
    let src = "#[cfg(test)] mod tests { fn t(b: &[u8]) { \
               capnp::serialize::read_message(b, Default::default()); } }";
    assert!(scan("crates/x/src/lib.rs", src).is_empty());
}
