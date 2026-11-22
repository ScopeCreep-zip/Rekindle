//! `check-log-scrub`: every log sink scrubs identifiers (rule B19).
//!
//! Two checks over production code (`src-tauri/src` and every
//! `crates/*/src`; `tests/` trees, `tests.rs`, `*_tests.rs` and items
//! under `#[cfg(test)]` or a test attribute are skipped):
//! - every statement that builds a tracing subscriber (a call through
//!   `tracing_subscriber::{fmt, registry, fmt::layer, fmt::init, …}` or
//!   to `set_global_default`) names
//!   `rekindle_utils::log_scrub::ScrubbingMakeWriter` in that same
//!   statement, so the writer cannot be swapped out of the chain;
//! - every binary (`src/main.rs`, `src/bin/*.rs`) calls
//!   `log_scrub::install_panic_hook` or `install_panic_hook_with`, itself
//!   or in the `run` function of its package's `lib.rs` that its `main`
//!   calls (the desktop's `rekindle_lib::run`).
//!
//! And a third over the same code: the debug-only shared-key variable
//! `REKINDLE_LOG_SCRUB_KEY` (plan RD2) is named only inside an item under
//! `#[cfg(debug_assertions)]`, so no release build reads it (the
//! compiler then rejects any release path that uses the gated name).

use std::path::Path;

use anyhow::{anyhow, Result};
use quote::ToTokens;
use syn::visit::Visit;

use crate::walk_source_files;

/// Last path segments that build a subscriber when reached through
/// `tracing_subscriber`.
const SUBSCRIBER_ROOTS: &[&str] = &["fmt", "registry", "layer", "init", "try_init", "builder"];
const WRITER: &str = "ScrubbingMakeWriter";
const PANIC_HOOKS: &[&str] = &["install_panic_hook", "install_panic_hook_with"];
const SHARED_KEY_VAR: &str = "REKINDLE_LOG_SCRUB_KEY";

fn is_debug_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Meta>()
                .is_ok_and(|m| m.path().is_ident("debug_assertions"))
    })
}

/// Finds `SHARED_KEY_VAR` named outside a `#[cfg(debug_assertions)]` item.
struct SharedKeyFinder<'a> {
    rel: &'a str,
    src: &'a str,
    cursor: usize,
    gated: usize,
    hits: Vec<String>,
}

impl SharedKeyFinder<'_> {
    fn within(&mut self, attrs: &[syn::Attribute], walk: impl FnOnce(&mut Self)) {
        if is_test_attr(attrs) {
            return;
        }
        let gate = usize::from(is_debug_only(attrs));
        self.gated += gate;
        walk(self);
        self.gated -= gate;
    }

    /// One mention, walked in source order: the line of the next quoted
    /// occurrence (spans carry no lines without `span-locations`).
    fn found(&mut self) {
        let quoted = format!("\"{SHARED_KEY_VAR}\"");
        let line = match self.src[self.cursor..].find(&quoted) {
            Some(at) => {
                let abs = self.cursor + at;
                self.cursor = abs + 1;
                self.src[..abs].matches('\n').count() + 1
            }
            None => 0,
        };
        if self.gated == 0 {
            self.hits.push(format!(
                "{}:{line}  {SHARED_KEY_VAR} named outside #[cfg(debug_assertions)]",
                self.rel
            ));
        }
    }
}

impl<'ast> Visit<'ast> for SharedKeyFinder<'_> {
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

    fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
        if lit.value() == SHARED_KEY_VAR {
            self.found();
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac
            .tokens
            .to_string()
            .contains(&format!("\"{SHARED_KEY_VAR}\""))
        {
            self.found();
        }
    }
}

fn scan_shared_key(rel: &str, src: &str) -> Vec<String> {
    let Ok(parsed) = syn::parse_file(src) else {
        return Vec::new();
    };
    let mut finder = SharedKeyFinder {
        rel,
        src,
        cursor: 0,
        gated: 0,
        hits: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.hits
}

fn is_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let path = a.path().to_token_stream().to_string();
        let text = a.to_token_stream().to_string();
        path.ends_with("test") || (path == "cfg" && text.contains("test"))
    })
}

fn is_subscriber_root(path: &syn::Path) -> bool {
    let names: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    let Some(last) = names.last() else {
        return false;
    };
    last == "set_global_default"
        || (names.iter().any(|n| n == "tracing_subscriber")
            && SUBSCRIBER_ROOTS.contains(&last.as_str()))
}

struct Finder<'a> {
    rel: &'a str,
    src: &'a str,
    cursor: usize,
    /// A subscriber root seen inside the statement being walked and not
    /// yet claimed by a deeper statement.
    pending_root: bool,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn hit(&mut self) {
        let line = match self.src[self.cursor..].find("tracing_subscriber") {
            Some(at) => {
                let abs = self.cursor + at;
                self.cursor = abs + 1;
                self.src[..abs].matches('\n').count() + 1
            }
            None => 0,
        };
        self.hits.push(format!(
            "{}:{line}  tracing subscriber built without log_scrub::{WRITER}",
            self.rel
        ));
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
        if !is_test_attr(attrs) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if !is_test_attr(&f.attrs) {
            syn::visit::visit_impl_item_fn(self, f);
        }
    }

    fn visit_stmt(&mut self, stmt: &'ast syn::Stmt) {
        let outer = std::mem::replace(&mut self.pending_root, false);
        syn::visit::visit_stmt(self, stmt);
        if self.pending_root && !stmt.to_token_stream().to_string().contains(WRITER) {
            self.hit();
        }
        self.pending_root = outer;
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*call.func {
            if is_subscriber_root(&p.path) {
                self.pending_root = true;
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn scan_subscribers(rel: &str, src: &str) -> Vec<String> {
    // A file that does not parse is cargo's problem to report.
    let Ok(parsed) = syn::parse_file(src) else {
        return Vec::new();
    };
    let mut finder = Finder {
        rel,
        src,
        cursor: 0,
        pending_root: false,
        hits: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.hits
}

fn calls_panic_hook(tokens: &str) -> bool {
    PANIC_HOOKS
        .iter()
        .any(|h| tokens.contains(&format!("{h} (")))
}

/// Whether the binary at `rel` installs the scrubbing panic hook, itself or
/// through its package's `lib.rs` `run` (`lib_src`).
fn binary_installs_hook(main_src: &str, lib_src: Option<&str>) -> bool {
    let Ok(main) = syn::parse_file(main_src) else {
        return true;
    };
    let main_tokens = main.to_token_stream().to_string();
    if calls_panic_hook(&main_tokens) {
        return true;
    }
    if !main_tokens.contains(":: run ()") {
        return false;
    }
    let Some(lib) = lib_src.and_then(|s| syn::parse_file(s).ok()) else {
        return false;
    };
    lib.items.iter().any(|item| match item {
        syn::Item::Fn(f) => {
            f.sig.ident == "run" && calls_panic_hook(&f.block.to_token_stream().to_string())
        }
        _ => false,
    })
}

fn is_test_file(rel: &str) -> bool {
    rel.contains("/tests/") || rel.ends_with("/tests.rs") || rel.ends_with("_tests.rs")
}

fn package_src_dirs(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut dirs = vec![root.join("src-tauri/src")];
    for entry in std::fs::read_dir(root.join("crates"))? {
        let src = entry?.path().join("src");
        if src.is_dir() {
            dirs.push(src);
        }
    }
    dirs.sort();
    Ok(dirs)
}

pub(crate) fn check_log_scrub(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for src_dir in package_src_dirs(root)? {
        for path in walk_source_files(&src_dir, &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if is_test_file(&rel) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            hits.extend(scan_subscribers(&rel, &src));
            hits.extend(scan_shared_key(&rel, &src));
            let is_binary =
                path == src_dir.join("main.rs") || path.parent() == Some(&src_dir.join("bin"));
            if is_binary {
                let lib = std::fs::read_to_string(src_dir.join("lib.rs")).ok();
                if !binary_installs_hook(&src, lib.as_deref()) {
                    hits.push(format!(
                        "{rel}  binary does not install log_scrub::install_panic_hook(_with)"
                    ));
                }
            }
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} log sink(s) bypass the scrubber (rule B19). Write through \
         `rekindle_utils::log_scrub::ScrubbingMakeWriter` and install \
         `log_scrub::install_panic_hook` in every binary; name \
         `{SHARED_KEY_VAR}` only under `#[cfg(debug_assertions)]`.",
        hits.len()
    ))
}

#[cfg(test)]
#[path = "log_scrub_tests.rs"]
mod tests;
