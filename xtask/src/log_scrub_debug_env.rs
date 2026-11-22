//! The third `check-log-scrub` check: a debug-only environment variable
//! (the real-app suite's switches, plan RV1; the shared scrubber key,
//! plan RD2) is named only inside an item
//! under `#[cfg(debug_assertions)]`, so no release build reads it (the
//! compiler then rejects any release path that uses the gated name).
//! Items under `#[cfg(test)]` or a test attribute are skipped, and so are
//! attributes (doc comments name the variable to explain it, never read it).

use quote::ToTokens;
use syn::visit::Visit;

use super::is_test_attr;

/// Every variable only a debug build may read.
pub(super) const DEBUG_ONLY_VARS: &[&str] =
    &["REKINDLE_ROUTE_BUDGET_CAPACITY", "REKINDLE_LOG_SCRUB_KEY"];

fn is_debug_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Meta>()
                .is_ok_and(|m| m.path().is_ident("debug_assertions"))
    })
}

fn named_var(text: &str) -> Option<&'static str> {
    DEBUG_ONLY_VARS.iter().copied().find(|v| text.contains(v))
}

/// Finds a [`DEBUG_ONLY_VARS`] name outside a `#[cfg(debug_assertions)]`
/// item.
struct Finder<'a> {
    rel: &'a str,
    src: &'a str,
    cursor: usize,
    gated: usize,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn within(&mut self, attrs: &[syn::Attribute], walk: impl FnOnce(&mut Self)) {
        if is_test_attr(attrs) {
            return;
        }
        let gate = usize::from(is_debug_only(attrs));
        self.gated += gate;
        walk(self);
        self.gated -= gate;
    }

    /// One mention of `var`, walked in source order: the line of its next
    /// occurrence (spans carry no lines without `span-locations`).
    fn found(&mut self, var: &str) {
        let mut line = 0;
        while let Some(at) = self.src[self.cursor..].find(var) {
            let abs = self.cursor + at;
            self.cursor = abs + 1;
            let start = self.src[..abs].rfind('\n').map_or(0, |i| i + 1);
            // A comment's mention is no item's (attributes are not walked).
            if !self.src[start..abs].trim_start().starts_with("//") {
                line = self.src[..abs].matches('\n').count() + 1;
                break;
            }
        }
        if self.gated == 0 {
            self.hits.push(format!(
                "{}:{line}  {var} named outside #[cfg(debug_assertions)]",
                self.rel
            ));
        }
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attrs = match item {
            syn::Item::Fn(f) => &f.attrs,
            syn::Item::Mod(m) => &m.attrs,
            syn::Item::Impl(i) => &i.attrs,
            syn::Item::Const(c) => &c.attrs,
            syn::Item::Static(s) => &s.attrs,
            _ => return syn::visit::visit_item(self, item),
        };
        self.within(attrs, |this| syn::visit::visit_item(this, item));
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.within(&f.attrs, |this| syn::visit::visit_impl_item_fn(this, f));
    }

    /// Attributes read nothing: a doc comment may name the variable.
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}

    fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
        if let Some(var) = named_var(&lit.value()) {
            self.found(var);
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(var) = named_var(&mac.tokens.to_token_stream().to_string()) {
            self.found(var);
        }
    }
}

/// Every mention of a debug-only variable in `src` outside debug-only code.
pub(super) fn scan(rel: &str, src: &str) -> Vec<String> {
    // A file that does not parse is cargo's problem to report.
    let Ok(parsed) = syn::parse_file(src) else {
        return Vec::new();
    };
    let mut finder = Finder {
        rel,
        src,
        cursor: 0,
        gated: 0,
        hits: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.hits
}

#[cfg(test)]
mod tests {
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
}
