//! Compile-fail tests via trybuild.
//!
//! Each file in tests/compile_fail/ is a standalone program that
//! MUST fail to compile. trybuild asserts the failure. This enforces
//! type-system invariants that are otherwise documentation-only.

#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
