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
//! `#[test]`/`#[tokio::test]` functions, and the files [`crate::test_code`] knows as test code by declaration)
//! is not scanned.
//!
//! In the Host's trees ([`HOST`]) blocking work goes through
//! `rekindle_lifecycle::run_blocking` too: a bare `spawn_blocking` or
//! `thread::spawn` there would finish in wall time, which the network
//! simulator cannot order (plan E2.B F11).

use std::path::Path;

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::test_code::is_test_attr;
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
    "crates/rekindle-outbox/src",
];

/// `(file, function, why it outlives every session)`.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "src-tauri/src/skin_scheme.rs",
        "handle",
        "one skin:// request's answer, before unlock too (the login skin); it holds no session \
         state and ends when the Host answers, which refuses library bytes once locked",
    ),
    (
        "src-tauri/src/windows/shape/macos.rs",
        "start",
        "one window's cursor tracker (plan SK0 S3): owned by the window, not a session; it reads \
         only the cursor and the window's silhouette, and stops on the window's Destroyed event",
    ),
    (
        "src-tauri/src/windows/shape/skin.rs",
        "refresh",
        "one SkinFrames answer for the window shaper (plan SK0 S3), before unlock too: owned by \
         the windows, not a session; it holds only the shown skin's validated frames and ends \
         when the Host answers",
    ),
    (
        "src-tauri/src/setup.rs",
        "*",
        "app-lifetime boot: node start, dispatch loop, deep links",
    ),
    (
        "src-tauri/src/stop_signals.rs",
        "install",
        "the process's stop-signal reader, installed before unlock: it turns SIGTERM/SIGINT into \
         the graceful quit that ends every session, so no session can own it",
    ),
    (
        "src-tauri/src/event_dispatch.rs",
        "*",
        "app-lifetime webview event queue",
    ),
    (
        "src-tauri/src/services/veilid/lifecycle/dispatch.rs",
        "start_dispatch_loop",
        "the app-lifetime dispatch loop's ingress workers; joined when the loop stops",
    ),
    (
        "crates/rekindle-node/src/host/serve.rs",
        "spawn_bus_subscriber",
        "host-lifetime bus subscriber; Host::shutdown joins its handle",
    ),
    (
        "crates/rekindle-node/src/host/serve.rs",
        "spawn_accept",
        "host-lifetime accept loop; Host::shutdown stops the server and ends it",
    ),
    (
        "crates/rekindle-transport/src/broadcast/node/lifecycle.rs",
        "*",
        "the TransportNode outlives every unlock; TransportNode::shutdown joins its loops",
    ),
];

/// The trees of the Host, whose blocking work runs through
/// `rekindle_lifecycle::run_blocking`.
const HOST: &[&str] = &[
    "crates/rekindle-node/src/",
    "crates/rekindle-transport/src/",
];

/// The free functions that start blocking work outside `run_blocking`.
const BLOCKING: &[&[&str]] = &[
    &["tokio", "task", "spawn_blocking"],
    &["tokio", "spawn_blocking"],
    &["task", "spawn_blocking"],
    &["thread", "spawn"],
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

    fn hit(&mut self, needle: &str, fix: &str) {
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
            "{}:{line}  {needle}(…) in `{function}` — {fix}",
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
                self.hit(&segments.join("::"), "spawn on a SessionScope");
            }
            let ends_with = |b: &&[&str]| {
                segments.len() >= b.len() && segments[segments.len() - b.len()..] == **b
            };
            if HOST.iter().any(|h| self.rel.starts_with(h)) && BLOCKING.iter().any(ends_with) {
                self.hit(
                    &segments.join("::"),
                    "run it through rekindle_lifecycle::run_blocking",
                );
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

pub(crate) fn check_bare_spawn(root: &Path) -> Result<()> {
    let tests = crate::test_code::TestCode::load(root)?;
    let mut hits = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    for dir in SCANNED {
        for path in walk_source_files(&root.join(dir), &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if tests.is_test(&path) {
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
        "{} bare spawn(s) outside a SessionScope or run_blocking (plan C4, rule B20; F11)",
        hits.len()
    ))
}

#[cfg(test)]
#[path = "bare_spawn_tests.rs"]
mod tests;
