use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{Root, TestCode};

fn code(files: &[(&str, &str)], roots: &[(&str, bool)]) -> TestCode {
    let sources: BTreeMap<PathBuf, String> = files
        .iter()
        .map(|(p, s)| (PathBuf::from(p), (*s).to_owned()))
        .collect();
    let roots: Vec<Root> = roots
        .iter()
        .map(|(p, test)| Root {
            path: PathBuf::from(p),
            test: *test,
        })
        .collect();
    TestCode::from_sources(&sources, &roots, vec![PathBuf::from("/c/tests")])
}

#[test]
fn a_cfg_test_mod_file_is_test_code() {
    let t = code(
        &[
            ("/c/src/lib.rs", "mod a; #[cfg(test)] mod x;"),
            ("/c/src/a.rs", ""),
            ("/c/src/x.rs", "mod helpers;"),
            ("/c/src/x/helpers.rs", ""),
        ],
        &[("/c/src/lib.rs", false)],
    );
    assert!(t.is_test(Path::new("/c/src/x.rs")));
    // A submodule of test code is test code, whatever its declaration says.
    assert!(t.is_test(Path::new("/c/src/x/helpers.rs")));
    assert!(!t.is_test(Path::new("/c/src/a.rs")));
    assert!(!t.is_test(Path::new("/c/src/lib.rs")));
}

#[test]
fn a_tests_named_file_declared_without_cfg_test_is_shipped() {
    let t = code(
        &[
            ("/c/src/lib.rs", "mod x; mod x_tests; mod tests;"),
            ("/c/src/x.rs", ""),
            ("/c/src/x_tests.rs", ""),
            ("/c/src/tests.rs", ""),
        ],
        &[("/c/src/lib.rs", false)],
    );
    assert!(!t.is_test(Path::new("/c/src/x_tests.rs")));
    assert!(!t.is_test(Path::new("/c/src/tests.rs")));
}

#[test]
fn a_path_declared_test_file_is_test_code() {
    let t = code(
        &[
            (
                "/c/src/lib.rs",
                "mod codec; #[cfg(test)] #[path = \"lib_tests.rs\"] mod tests;",
            ),
            ("/c/src/lib_tests.rs", "#[path = \"lib_tests_fixtures.rs\"] mod fixtures;"),
            ("/c/src/lib_tests_fixtures.rs", ""),
            (
                "/c/src/codec.rs",
                "mod inner { #[cfg(test)] mod deep; } #[cfg(test)] #[path = \"codec_tests.rs\"] mod tests;",
            ),
            ("/c/src/codec/inner/deep.rs", ""),
            ("/c/src/codec_tests.rs", ""),
        ],
        &[("/c/src/lib.rs", false)],
    );
    assert!(t.is_test(Path::new("/c/src/lib_tests.rs")));
    assert!(t.is_test(Path::new("/c/src/lib_tests_fixtures.rs")));
    assert!(t.is_test(Path::new("/c/src/codec/inner/deep.rs")));
    assert!(t.is_test(Path::new("/c/src/codec_tests.rs")));
    assert!(!t.is_test(Path::new("/c/src/codec.rs")));
    assert_eq!(
        t.declarer(Path::new("/c/src/codec_tests.rs")),
        Some(Path::new("/c/src/codec.rs"))
    );
}

#[test]
fn test_targets_and_their_directories_are_test_code() {
    let t = code(
        &[
            ("/c/tests/it.rs", "mod common;"),
            ("/c/tests/common/mod.rs", ""),
            ("/c/src/lib.rs", ""),
            ("/c/src/bench_main.rs", "mod util;"),
            ("/c/src/util.rs", ""),
        ],
        &[
            ("/c/tests/it.rs", true),
            ("/c/src/lib.rs", false),
            ("/c/src/bench_main.rs", true),
        ],
    );
    assert!(t.is_test(Path::new("/c/tests/it.rs")));
    assert!(t.is_test(Path::new("/c/tests/common/mod.rs")));
    assert!(t.is_test(Path::new("/c/src/bench_main.rs")));
    // A crate root owns its directory, so its submodules sit beside it.
    assert!(t.is_test(Path::new("/c/src/util.rs")));
    assert!(!t.is_test(Path::new("/c/src/lib.rs")));
}

#[test]
fn a_file_declared_from_shipped_code_is_shipped_even_if_tests_also_load_it() {
    let t = code(
        &[
            (
                "/c/src/lib.rs",
                "#[cfg(test)] #[path = \"shared.rs\"] mod a; #[path = \"shared.rs\"] mod b;",
            ),
            ("/c/src/shared.rs", ""),
        ],
        &[("/c/src/lib.rs", false)],
    );
    assert!(!t.is_test(Path::new("/c/src/shared.rs")));
}
