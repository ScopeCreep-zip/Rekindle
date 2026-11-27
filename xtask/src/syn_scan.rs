//! The skeleton every `syn` source check shares (gate hardening item 2,
//! triage b7: "the gate must not fail itself"): run a finder over a parsed
//! file, locate a hit's line by its text, skip test items, and print the
//! hits as the verdict. Each check keeps only what it looks for.

use std::path::Path;

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::walk::read_files_under;

/// Runs `finder` over `src`, or returns `None` when `src` does not parse (a
/// file that does not parse is cargo's problem to report).
pub(crate) fn visit<V: for<'ast> Visit<'ast>>(src: &str, mut finder: V) -> Option<V> {
    let parsed = syn::parse_file(src).ok()?;
    finder.visit_file(&parsed);
    Some(finder)
}

/// Locates hits by their text: each needle is searched past the previous
/// hit, so repeated occurrences report their own lines (no `span-locations`
/// on proc-macro2; see xtask/Cargo.toml).
pub(crate) struct LineCursor<'a> {
    src: &'a str,
    at: usize,
}

impl<'a> LineCursor<'a> {
    pub(crate) fn new(src: &'a str) -> Self {
        Self { src, at: 0 }
    }

    /// The 1-based line of the next `needle` (0 when it is not found).
    pub(crate) fn line(&mut self, needle: &str) -> usize {
        match self.src[self.at..].find(needle) {
            Some(off) => {
                let abs = self.at + off;
                self.at = abs + needle.len();
                self.src[..abs].matches('\n').count() + 1
            }
            None => 0,
        }
    }

    /// The 1-based line of the next `needle` that is not in a `//` comment
    /// (0 when there is none).
    pub(crate) fn line_outside_comments(&mut self, needle: &str) -> usize {
        while let Some(off) = self.src[self.at..].find(needle) {
            let abs = self.at + off;
            self.at = abs + 1;
            let start = self.src[..abs].rfind('\n').map_or(0, |i| i + 1);
            if !self.src[start..abs].trim_start().starts_with("//") {
                return self.src[..abs].matches('\n').count() + 1;
            }
        }
        0
    }
}

/// Passes when `hits` is empty; otherwise prints each hit and fails with
/// "`n` `what`".
pub(crate) fn verdict(hits: &[String], what: &str) -> Result<()> {
    if hits.is_empty() {
        return Ok(());
    }
    for h in hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!("{} {what}", hits.len()))
}

/// The `visit_item_mod` and `visit_item_fn` of a finder's `Visit` impl that
/// skip test items (`crate::test_code::is_test_attr`).
macro_rules! skip_test_items {
    ($ast:lifetime) => {
        $crate::syn_scan::skip_test_items!($ast, visit_item_mod: ItemMod, visit_item_fn: ItemFn);
    };
    ($ast:lifetime, $($visit:ident: $node:ident),+) => {
        $(
            fn $visit(&mut self, item: &$ast syn::$node) {
                if !item.attrs.iter().any($crate::test_code::is_test_attr) {
                    syn::visit::$visit(self, item);
                }
            }
        )+
    };
}

pub(crate) use skip_test_items;

/// The hits of `scan` over every readable `.rs` file under `dirs`
/// (relative to `root`), each named relative to `root`.
pub(crate) fn scan_rust_dirs(
    root: &Path,
    dirs: &[&str],
    scan: impl Fn(&str, &str) -> Vec<String>,
) -> Result<Vec<String>> {
    let mut hits = Vec::new();
    for dir in dirs {
        for (file, src) in read_files_under(root, &root.join(dir), &["rs"])? {
            hits.extend(scan(&file.rel, &src));
        }
    }
    Ok(hits)
}
