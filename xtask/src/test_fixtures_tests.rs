use std::fmt::Write;

use super::{is_rust_test_file, scan_rust, scan_ts};

/// A high-entropy hex value built at run time, so no secret-shaped literal sits in this file.
fn sample() -> String {
    let mut out = String::new();
    for i in 0u32..32 {
        write!(out, "{:02x}", (i * 73 + 41) % 251).unwrap();
    }
    out
}

fn count(rel: &str, src: &str) -> usize {
    scan_rust(rel, src).unwrap().len()
}

#[test]
fn a_credential_literal_in_a_test_fails() {
    let src = format!("#[test]\nfn t() {{ let key = \"{}\"; }}", sample());
    assert_eq!(count("crates/x/src/lib.rs", &src), 1);
}

#[test]
fn a_copy_of_a_canonical_value_fails_and_its_path_passes() {
    let copy = format!(
        "#[test]\nfn t() {{ let key = \"{}\"; }}",
        rekindle_test_fixtures::HEX_KEY_64
    );
    assert_eq!(count("crates/x/src/lib.rs", &copy), 1);
    let path = "#[test]\nfn t() { let key = rekindle_test_fixtures::HEX_KEY_64; }";
    assert_eq!(count("crates/x/src/lib.rs", path), 0);
}

#[test]
fn production_code_plain_names_and_low_entropy_pass() {
    let s = sample();
    let prod = format!("fn t() {{ let key = \"{s}\"; }}");
    assert_eq!(count("crates/x/src/lib.rs", &prod), 0);
    let plain = format!("#[test]\nfn t() {{ let id = \"{s}\"; }}");
    assert_eq!(count("crates/x/src/lib.rs", &plain), 0);
    let flat = "#[test]\nfn t() { let key = \"aaaaaaaaaaaa1\"; }";
    assert_eq!(count("crates/x/src/lib.rs", flat), 0);
}

#[test]
fn fields_assignments_and_macro_pairs_fail() {
    let s = sample();
    let src = format!(
        "fn f() {{ let v = S {{ token: \"{s}\".into() }}; x.api_key = \"{s}\".to_string(); \
         let j = json!({{\"secret\": \"{s}\"}}); }}"
    );
    assert_eq!(count("crates/x/tests/a.rs", &src), 3);
}

#[test]
fn cfg_test_modules_are_scanned_and_cfg_not_test_is_not() {
    let s = sample();
    let on = format!("#[cfg(test)]\nmod tests {{ const SEED: &str = \"{s}\"; }}");
    assert_eq!(count("crates/x/src/lib.rs", &on), 1);
    let off = format!("#[cfg(not(test))]\nmod m {{ const SEED: &str = \"{s}\"; }}");
    assert_eq!(count("crates/x/src/lib.rs", &off), 0);
}

#[test]
fn a_private_key_header_fails_anywhere_in_test_code() {
    let pem = ["-----BEGIN", " EC PRIVATE", " KEY-----"].concat();
    let src = format!("#[test]\nfn t() {{ let body = \"{pem}\"; }}");
    assert_eq!(count("crates/x/src/lib.rs", &src), 1);
}

#[test]
fn ts_credential_literals_fail_and_run_time_values_pass() {
    let src = format!(
        "const apiKey = \"{}\";\nconst me = {{ passphrase: randomBytes(18).toString(\"base64url\") }};\n\
         // don't pair this apostrophe across lines\nconst id = '{}';",
        sample(),
        sample()
    );
    assert_eq!(scan_ts(&src).len(), 1);
}

#[test]
fn rust_test_files_are_recognised() {
    assert!(is_rust_test_file("crates/x/tests/a.rs"));
    assert!(is_rust_test_file("crates/x/src/a_tests.rs"));
    assert!(is_rust_test_file("crates/x/src/a/tests.rs"));
    assert!(is_rust_test_file(
        "crates/rekindle-node/src/host/test_fixtures.rs"
    ));
    assert!(!is_rust_test_file("crates/x/src/testing.rs"));
}
