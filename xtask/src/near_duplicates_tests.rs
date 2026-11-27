//! The near-duplicate pass against the E4.1 S5 voice-adapter port, which the
//! byte-identical checks let through (reproduced from history: the desktop
//! adapter before `f5227664c` and the Host adapter it became), against an
//! unrelated module, and on small Type-2 cases.

use std::collections::BTreeMap;

use super::tokens::{normalise, renamed_hash, Interner, Tok};
use super::{scan, Finding, THRESHOLDS};

const S5_DESKTOP_DEPS: &str =
    include_str!("../fixtures/near_duplicates/s5_desktop_deps.rs.fixture");
const S5_HOST_DEPS: &str = include_str!("../fixtures/near_duplicates/s5_host_deps.rs.fixture");
const S5_DESKTOP_SETUP: &str =
    include_str!("../fixtures/near_duplicates/s5_desktop_setup.rs.fixture");
const S5_HOST_SETUP: &str = include_str!("../fixtures/near_duplicates/s5_host_setup.rs.fixture");
const UNRELATED: &str =
    include_str!("../fixtures/near_duplicates/unrelated_quality_cache.rs.fixture");

fn run(pairs: &[(&str, &str)]) -> Vec<Finding> {
    let sources: BTreeMap<String, String> = pairs
        .iter()
        .map(|(p, s)| ((*p).to_owned(), (*s).to_owned()))
        .collect();
    let found = scan(&sources, &THRESHOLDS).expect("fixtures parse");
    for f in &found {
        println!("{f}");
    }
    found
}

fn file_similarity(found: &[Finding]) -> Option<f64> {
    found
        .iter()
        .find(|f| f.kind == "file")
        .map(|f| f.similarity)
}

#[test]
fn the_s5_deps_port_fails_as_a_ported_file() {
    let found = run(&[
        ("a/deps_impl.rs", S5_DESKTOP_DEPS),
        ("b/deps.rs", S5_HOST_DEPS),
    ]);
    let sim = file_similarity(&found).expect("the S5 deps port is reported as a file pair");
    println!(
        "S5 deps file similarity {sim:.3} vs threshold {:.3}",
        THRESHOLDS.file_similarity
    );
    assert!(sim >= THRESHOLDS.file_similarity);
}

#[test]
fn the_s5_session_setup_port_fails_as_a_ported_file() {
    let found = run(&[
        ("a/session_setup.rs", S5_DESKTOP_SETUP),
        ("b/setup.rs", S5_HOST_SETUP),
    ]);
    let sim = file_similarity(&found).expect("the S5 setup port is reported as a file pair");
    println!(
        "S5 setup file similarity {sim:.3} vs threshold {:.3}",
        THRESHOLDS.file_similarity
    );
    assert!(sim >= THRESHOLDS.file_similarity);
}

#[test]
fn an_unrelated_module_passes() {
    let found = run(&[
        ("a/deps_impl.rs", S5_DESKTOP_DEPS),
        ("b/quality_cache.rs", UNRELATED),
    ]);
    assert!(found.is_empty(), "no clone between unrelated modules");
    let found = run(&[
        ("a/deps.rs", S5_HOST_DEPS),
        ("b/quality_cache.rs", UNRELATED),
    ]);
    assert!(found.is_empty(), "no clone between unrelated modules");
}

/// Two modules of one domain and one author that are not a port of each
/// other (the desktop session setup and the Host deps) share idioms, not
/// code: they pass (file similarity 0.024 at the calibration).
#[test]
fn same_domain_modules_that_are_not_a_port_pass() {
    let found = run(&[
        ("a/session_setup.rs", S5_DESKTOP_SETUP),
        ("b/deps.rs", S5_HOST_DEPS),
    ]);
    assert!(found.iter().all(|f| f.kind != "file"), "not a ported file");
    let found = run(&[
        ("a/deps_impl.rs", S5_DESKTOP_DEPS),
        ("b/setup.rs", S5_HOST_SETUP),
    ]);
    assert!(found.iter().all(|f| f.kind != "file"), "not a ported file");
}

const ORIGINAL: &str = r#"
fn total_weight(items: &[Item], limit: u32) -> Result<u32, Error> {
    let mut sum = 0u32;
    for item in items.iter().filter(|i| i.enabled) {
        let part = item.weight.checked_mul(item.count).ok_or(Error::Overflow)?;
        sum = sum.checked_add(part).ok_or(Error::Overflow)?;
        if sum > limit {
            return Err(Error::OverLimit { total: sum, cap: limit });
        }
    }
    tracing::debug!(sum, "weights summed");
    Ok(sum)
}
"#;

/// `ORIGINAL` with every local, the function and the error renamed and a
/// literal changed: a Type-2 clone.
const RENAMED: &str = r#"
fn mass_of(things: &[Thing], bound: u32) -> Result<u32, Fault> {
    let mut acc = 1u32;
    for t in things.iter().filter(|x| x.enabled) {
        let w = t.weight.checked_mul(t.count).ok_or(Fault::Overflow)?;
        acc = acc.checked_add(w).ok_or(Fault::Overflow)?;
        if acc > bound {
            return Err(Fault::OverLimit { total: acc, cap: bound });
        }
    }
    log::debug!(acc, "masses summed");
    Ok(acc)
}
"#;

#[test]
fn a_renamed_function_is_a_clone() {
    let found = run(&[("a.rs", ORIGINAL), ("b.rs", RENAMED)]);
    assert!(
        found
            .iter()
            .any(|f| f.kind == "function" && f.similarity >= 0.99),
        "a Type-2 rename is a function clone"
    );
}

fn toks(src: &str) -> Vec<Tok> {
    let mut int = Interner::default();
    let mut out = Vec::new();
    normalise(src.parse().expect("tokens"), &mut int, &mut out);
    out
}

#[test]
fn renaming_is_consistent() {
    assert_eq!(
        renamed_hash(&toks("d = a + b;")),
        renamed_hash(&toks("s = p + q;"))
    );
    assert_ne!(
        renamed_hash(&toks("d = a + b;")),
        renamed_hash(&toks("x = x + x;"))
    );
}

#[test]
fn paths_attributes_and_literals_normalise() {
    assert_eq!(
        renamed_hash(&toks("#[allow(x)] crate::a::b::call(1, \"s\");")),
        renamed_hash(&toks("other::call(7, \"t\");"))
    );
}
