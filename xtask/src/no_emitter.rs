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

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::walk_source_files;

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
    src: &'a str,
    /// Byte offset past the previous hit's text, so repeated occurrences
    /// of the same text report their own lines (no `span-locations` on
    /// proc-macro2; see xtask/Cargo.toml).
    cursor: usize,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn hit(&mut self, needle: &str, what: &str) {
        let line = match self.src[self.cursor..].find(needle) {
            Some(at) => {
                let abs = self.cursor + at;
                self.cursor = abs + needle.len();
                self.src[..abs].matches('\n').count() + 1
            }
            None => 0,
        };
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
    // A file that does not parse is cargo's problem to report.
    let Ok(parsed) = syn::parse_file(src) else {
        return Vec::new();
    };
    let mut finder = Finder {
        rel,
        src,
        cursor: 0,
        hits: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.hits
}

pub(crate) fn check_no_emitter(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for path in walk_source_files(&root.join("src-tauri/src"), &["rs"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        hits.extend(scan(&rel, &src));
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} Tauri Emitter use(s) in src-tauri. Send webview events with \
         `event_dispatch::{{emit, emit_journaled, emit_subscription, …}}`.",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::scan;

    #[test]
    fn flags_emitter_imports_and_method_calls() {
        let bad = r#"
            use tauri::{Emitter, Manager};
            fn f(app: tauri::AppHandle, w: tauri::WebviewWindow) {
                let _ = app.emit("x", ());
                let _ = w.emit_to("buddy-list", "x", ());
                let _ = <tauri::AppHandle as tauri::Emitter>::emit_str(&app, "x", String::new());
            }
        "#;
        let hits = scan("f.rs", bad);
        assert_eq!(hits.len(), 4, "{hits:?}");
        assert!(hits[0].starts_with("f.rs:2 "), "{hits:?}");
    }

    #[test]
    fn allows_the_router_entry_points() {
        let ok = r"
            fn f(app: &tauri::AppHandle) {
                crate::event_dispatch::emit(app, crate::event_dispatch::WebviewEvent::ProfileUpdated);
                emit_journaled(state, event);
                channel.send(envelope);
            }
        ";
        assert!(scan("f.rs", ok).is_empty());
    }
}
