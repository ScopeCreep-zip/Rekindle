//! The structural floor and the stripped jscpd input on list and logic
//! shapes (gate hardening item 2, "Item 2 rulings after the triage").

use super::{LexedText, MIN_STRUCTURAL};
use crate::structural::test_fixture_sources::{LINT_LIST, VARIANT_LIST};
use crate::token_clones::strip_rust;

#[test]
fn a_lint_list_strips_to_nothing_and_keeps_its_lines() {
    let stripped = strip_rust(LINT_LIST);
    assert_eq!(stripped.lines().count(), LINT_LIST.lines().count());
    assert!(!stripped.contains("clippy"), "{stripped}");
    assert!(stripped.contains("pub mod pool;"));
    assert_eq!(LexedText::new(&stripped).structural(3, 16), 0);
}

#[test]
fn a_variant_list_and_an_or_pattern_are_under_the_floor() {
    let text = LexedText::new(VARIANT_LIST);
    assert!(text.structural(6, 24) < MIN_STRUCTURAL, "the variant list");
    assert!(text.structural(28, 60) < MIN_STRUCTURAL, "the or-pattern");
}

#[test]
fn logic_is_over_the_floor() {
    let src = "fn f(a: u32) -> u32 { if a > 2 { return a * 3 + 1; } g(a) - 4 }";
    assert!(LexedText::new(src).structural(1, 1) >= MIN_STRUCTURAL);
}

#[test]
fn use_items_and_attributes_strip_where_they_are_items() {
    let src = "use a::{b,\n    c};\npub(crate) use d;\n#[derive(Debug)]\nstruct S;\n\
               fn f() -> impl Sized + use<'a> { use e::g; let x = h(1); x }\n";
    let stripped = strip_rust(src);
    assert_eq!(stripped.lines().count(), src.lines().count());
    assert!(!stripped.contains("a::{b") && !stripped.contains(" d;"));
    assert!(!stripped.contains("derive") && !stripped.contains("e::g"));
    assert!(stripped.contains("struct S;"));
    assert!(
        stripped.contains("use<'a>"),
        "precise capturing is not an item"
    );
    assert!(stripped.contains("let x = h(1); x"));
}
