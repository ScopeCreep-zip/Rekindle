//! `check-no-emitter`: `src-tauri` sends webview events only through
//! `event_dispatch` and the per-window `event_router` (ADR 0007, rule B17).
//!
//! Tauri's `Emitter` (`emit`, `emit_to`, `emit_filter`, `emit_str*`)
//! delivers to every webview that called the global JS `listen`, whatever
//! target it names, so one call site bypassing the router leaks a
//! conversation into every window. This gate rejects, anywhere under
//! `src-tauri/src`:
//! - a `use` of anything named `Emitter`;
//! - a method call named `emit`, `emit_to`, `emit_filter`, `emit_str`,
//!   `emit_str_to` or `emit_str_filter`.
//!
//! Free functions such as `event_dispatch::emit` are calls, not method
//! calls, and are allowed: they are the router's own entry points.

use std::path::Path;

use anyhow::Result;
use syn::visit::Visit;

use crate::syn_scan::{scan_rust_dirs, verdict, visit, LineCursor};

const EMITTER_METHODS: &[&str] = &[
    "emit",
    "emit_to",
    "emit_filter",
    "emit_str",
    "emit_str_to",
    "emit_str_filter",
];

struct Finder<'a> {
    rel: &'a str,
    lines: LineCursor<'a>,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn hit(&mut self, needle: &str, what: &str) {
        let line = self.lines.line(needle);
        self.hits.push(format!("{}:{line}  {what}", self.rel));
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
        match tree {
            syn::UseTree::Name(n) if n.ident == "Emitter" => {
                self.hit("Emitter", "use of tauri::Emitter");
            }
            syn::UseTree::Rename(r) if r.ident == "Emitter" => {
                self.hit("Emitter", "use of tauri::Emitter");
            }
            _ => {}
        }
        syn::visit::visit_use_tree(self, tree);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        if EMITTER_METHODS.contains(&name.as_str()) {
            self.hit(
                &format!(".{name}("),
                &format!(".{name}(…) — Tauri Emitter call"),
            );
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path.segments.iter().any(|s| s.ident == "Emitter") {
            self.hit("Emitter", "path through tauri::Emitter");
        }
        syn::visit::visit_path(self, path);
    }
}

fn scan(rel: &str, src: &str) -> Vec<String> {
    visit(
        src,
        Finder {
            rel,
            lines: LineCursor::new(src),
            hits: Vec::new(),
        },
    )
    .map_or_else(Vec::new, |f| f.hits)
}

pub(crate) fn check_no_emitter(root: &Path) -> Result<()> {
    let hits = scan_rust_dirs(root, &["src-tauri/src"], scan)?;
    verdict(
        &hits,
        "Tauri Emitter use(s) in src-tauri. Send webview events with \
         `event_dispatch::{emit, emit_journaled, emit_subscription, …}`.",
    )
}

#[cfg(test)]
#[path = "no_emitter_tests.rs"]
mod tests;
