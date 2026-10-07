//! `check-bare-spawn`: session work spawns through a `SessionScope`
//! (plan C4, rule B20).
//!
//! A task spawned with `tokio::spawn`, `tokio::task::spawn` or
//! `tauri::async_runtime::spawn` belongs to nobody: logout, lock and exit
//! cannot stop it, so it keeps running with the session's keys and state
//! after the session ended. Every such call in the trees that own session
//! work fails this gate unless its function is listed in [`ALLOWED`] with
//! the reason it outlives every session. Owned spawns (`JoinSet::spawn`,
//! `TaskTracker::spawn`) are method calls and are not matched.
//!
//! The allowlist names a function, not a line, so it does not go stale
//! when the file around it changes; an entry that no longer matches any
//! spawn fails the gate too, so the list cannot rot. Test code (`#[cfg(test)]` modules,
//! `#[test]`/`#[tokio::test]` functions, `tests/` and `*_tests.rs` files)
//! is not scanned.

use std::path::Path;

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::walk_source_files;

/// The trees whose tasks belong to a session.
const SCANNED: &[&str] = &[
    "src-tauri/src",
    "crates/rekindle-node/src",
    "crates/rekindle-transport/src",
    "crates/rekindle-presence/src",
    "crates/rekindle-friendship/src",
    "crates/rekindle-gossip/src",
    "crates/rekindle-voice/src",
    "crates/rekindle-calls/src",
    "crates/rekindle-video/src",
    "crates/rekindle-game-detect/src",
];

/// `(file, function, why it outlives every session)`.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "src-tauri/src/setup.rs",
        "*",
        "app-lifetime boot: node start, dispatch loop, deep links",
    ),
    (
        "src-tauri/src/event_dispatch.rs",
        "*",
        "app-lifetime webview event queue",
    ),
    (
        "src-tauri/src/services/session.rs",
        "begin",
        "the panic hook ends the login scope itself, so it cannot run in it",
    ),
    (
        "src-tauri/src/services/veilid/lifecycle/dispatch.rs",
        "start_dispatch_loop",
        "the app-lifetime dispatch loop's ingress workers; joined when the loop stops",
    ),
    (
        "crates/rekindle-node/src/host/mod.rs",
        "spawn_bus_subscriber",
        "process-lifetime bus subscriber; its handle is joined at exit",
    ),
    (
        "crates/rekindle-transport/src/broadcast/node/lifecycle.rs",
        "*",
        "the TransportNode outlives every unlock; TransportNode::shutdown joins its loops",
    ),
    (
        "crates/rekindle-transport/src/operations/calls/mod.rs",
        "spawn_timeout",
        "per-call timer whose handle the timers map owns and aborts on replace or drop",
    ),
];

/// The free functions that spawn an unowned task.
const BARE: &[&[&str]] = &[
    &["tokio", "spawn"],
    &["tokio", "task", "spawn"],
    &["tauri", "async_runtime", "spawn"],
    &["async_runtime", "spawn"],
];

struct Finder<'a> {
    rel: &'a str,
    src: &'a str,
    /// Byte offset past the previous hit's text, so repeated occurrences
    /// of the same text report their own lines (no `span-locations` on
    /// proc-macro2; see xtask/Cargo.toml).
    cursor: usize,
    /// Enclosing function names, innermost last.
    fns: Vec<String>,
    hits: Vec<String>,
    /// Indexes into [`ALLOWED`] that excused a spawn.
    used: std::collections::BTreeSet<usize>,
}

fn is_test_attr(attr: &syn::Attribute) -> bool {
    let path = attr.path();
    if path.is_ident("test") || path.segments.last().is_some_and(|s| s.ident == "test") {
        return true;
    }
    path.is_ident("cfg")
        && attr
            .parse_args::<syn::Meta>()
            .is_ok_and(|meta| meta.path().is_ident("test"))
}

impl Finder<'_> {
    fn allowed(&mut self) -> bool {
        let function = self.fns.last().map_or("", String::as_str);
        let entry = ALLOWED
            .iter()
            .position(|(file, f, _)| *file == self.rel && (*f == "*" || *f == function));
        entry.inspect(|i| {
            self.used.insert(*i);
        });
        entry.is_some()
    }

    fn hit(&mut self, needle: &str) {
        let line = match self.src[self.cursor..].find(needle) {
            Some(at) => {
                let abs = self.cursor + at;
                self.cursor = abs + needle.len();
                self.src[..abs].matches('\n').count() + 1
            }
            None => 0,
        };
        let function = self.fns.last().map_or("<top>", String::as_str);
        self.hits.push(format!(
            "{}:{line}  {needle}(…) in `{function}` — spawn on a SessionScope",
            self.rel
        ));
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !item.attrs.iter().any(is_test_attr) {
            syn::visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item.attrs.iter().any(is_test_attr) {
            return;
        }
        self.fns.push(item.sig.ident.to_string());
        syn::visit::visit_item_fn(self, item);
        self.fns.pop();
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if item.attrs.iter().any(is_test_attr) {
            return;
        }
        self.fns.push(item.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, item);
        self.fns.pop();
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            let segments: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            let bare = BARE
                .iter()
                .any(|b| segments.len() >= b.len() && segments[segments.len() - b.len()..] == **b);
            if bare && !self.allowed() {
                self.hit(&segments.join("::"));
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

/// The bare spawns in `src`, and the [`ALLOWED`] entries that excused one.
fn scan(rel: &str, src: &str) -> (Vec<String>, std::collections::BTreeSet<usize>) {
    // A file that does not parse is cargo's problem to report.
    let Ok(parsed) = syn::parse_file(src) else {
        return Default::default();
    };
    let mut finder = Finder {
        rel,
        src,
        cursor: 0,
        fns: Vec::new(),
        hits: Vec::new(),
        used: std::collections::BTreeSet::new(),
    };
    finder.visit_file(&parsed);
    (finder.hits, finder.used)
}

fn is_test_file(rel: &str) -> bool {
    rel.contains("/tests/") || rel.ends_with("_tests.rs") || rel.ends_with("/tests.rs")
}

pub(crate) fn check_bare_spawn(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    for dir in SCANNED {
        for path in walk_source_files(&root.join(dir), &["rs"])? {
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
            let (file_hits, file_used) = scan(&rel, &src);
            hits.extend(file_hits);
            used.extend(file_used);
        }
    }
    for (i, (file, function, _)) in ALLOWED.iter().enumerate() {
        if !used.contains(&i) {
            hits.push(format!(
                "{file}  allowlist entry `{function}` matches no spawn — remove it"
            ));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        eprintln!("    {h}");
    }
    Err(anyhow!(
        "{} bare spawn(s) outside a SessionScope (plan C4, rule B20)",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_bare_spawns_and_names_the_function() {
        let src = "fn work() { tokio::spawn(async {}); tauri::async_runtime::spawn(async {}); }";
        let (hits, _) = scan("src-tauri/src/x.rs", src);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits[0].contains("x.rs:1") && hits[0].contains("`work`"));
    }

    #[test]
    fn ignores_owned_spawns_and_test_code() {
        let src = r"
            fn owned(set: &mut tokio::task::JoinSet<()>) { set.spawn(async {}); }
            #[cfg(test)]
            mod tests { fn t() { tokio::spawn(async {}); } }
            #[tokio::test]
            async fn t2() { tokio::spawn(async {}); }
        ";
        assert!(scan("src-tauri/src/x.rs", src).0.is_empty());
    }

    #[test]
    fn allowlist_is_per_function() {
        let src = "fn begin() { tauri::async_runtime::spawn(async {}); }
                   fn other() { tauri::async_runtime::spawn(async {}); }";
        let (hits, used) = scan("src-tauri/src/services/session.rs", src);
        assert_eq!(hits.len(), 1);
        assert_eq!(used.len(), 1);
        assert!(hits[0].contains("`other`"));
    }
}
