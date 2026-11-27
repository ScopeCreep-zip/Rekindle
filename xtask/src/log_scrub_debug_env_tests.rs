use super::scan;

#[test]
fn a_release_mention_of_a_debug_only_var_is_flagged() {
    let bad = r#"
            /// Names REKINDLE_ROUTE_BUDGET_CAPACITY.
            const CAPACITY_VAR: &str = "REKINDLE_ROUTE_BUDGET_CAPACITY";
            fn read() -> Option<String> {
                tracing::warn!("seeded from REKINDLE_ROUTE_BUDGET_CAPACITY");
                std::env::var("REKINDLE_ROUTE_BUDGET_CAPACITY").ok()
            }
        "#;
    let hits = scan("crates/x/src/lib.rs", bad);
    assert_eq!(hits.len(), 3, "{hits:?}");
    assert!(hits[0].starts_with("crates/x/src/lib.rs:3 "), "{hits:?}");
}

#[test]
fn a_debug_only_or_test_mention_passes() {
    let good = r#"
            //! Debug builds read `REKINDLE_ROUTE_BUDGET_CAPACITY`.
            /// Seeded by `REKINDLE_ROUTE_BUDGET_CAPACITY` in a debug build.
            pub struct Budget;
            #[cfg(debug_assertions)]
            const CAPACITY_VAR: &str = "REKINDLE_ROUTE_BUDGET_CAPACITY";
            #[cfg(debug_assertions)]
            fn read() -> Option<String> {
                tracing::warn!("seeded from REKINDLE_ROUTE_BUDGET_CAPACITY");
                std::env::var("REKINDLE_ROUTE_BUDGET_CAPACITY").ok()
            }
            #[cfg(test)]
            mod tests {
                const V: &str = "REKINDLE_ROUTE_BUDGET_CAPACITY";
            }
        "#;
    assert!(scan("crates/x/src/lib.rs", good).is_empty());
}

/// RD2's shared scrubber key, the same assertions as its own check had.
#[test]
fn the_shared_key_is_named_only_in_debug_builds() {
    let gated = r#"
        #[cfg(debug_assertions)]
        const SHARED_KEY_VAR: &str = "REKINDLE_LOG_SCRUB_KEY";
        #[cfg(debug_assertions)]
        fn draw() { let _ = std::env::var("REKINDLE_LOG_SCRUB_KEY"); }
    "#;
    assert!(scan("f.rs", gated).is_empty());
    let open = r#"
        const SHARED_KEY_VAR: &str = "REKINDLE_LOG_SCRUB_KEY";
        #[cfg(not(debug_assertions))]
        fn draw() { let _ = std::env::var("REKINDLE_LOG_SCRUB_KEY"); }
        fn say() { println!("REKINDLE_LOG_SCRUB_KEY"); }
    "#;
    let hits = scan("f.rs", open);
    assert_eq!(hits.len(), 3, "{hits:?}");
    assert!(hits[0].starts_with("f.rs:2 "), "{hits:?}");
}
