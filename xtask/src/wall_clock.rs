//! `check-wall-clock`: code that runs inside the network simulator reads
//! no wall clock (plan E2.B, ADR 0021).
//!
//! The netsim (`crates/rekindle-node/tests/netsim/`, `crates/rekindle-netsim`)
//! runs the production Host on a paused tokio clock and a seeded RNG, and
//! one seed must give one trace (`scenarios::one_seed_one_trace`). A wall
//! clock read breaks that: the paused runtime cannot order it. This gate
//! fails on, in non-test code:
//!
//! - `std::time::Instant::now` (also `Instant::now` / `time::Instant::now`
//!   when the file imports std's `Instant` or `time`) — use
//!   `tokio::time::Instant`;
//! - `SystemTime::now`, `chrono::Utc::now`, `chrono::Local::now` — a
//!   timestamp that goes on the wire or into the database comes from the
//!   clock port (`rekindle_utils::time::Clock`, `ctx.clock`), because
//!   paused tokio time has no epoch;
//! - `std::thread::sleep` — use `tokio::time::sleep`;
//! - `rekindle_utils::{timestamp_ms, timestamp_secs, …}` — the free
//!   wall-clock helpers `SystemClock` is built on; pure code takes a `now`
//!   from its caller, everything else reads `ctx.clock`. A `pub use` of
//!   one fails too, so no crate re-exports it under its own path or a
//!   new name;
//! - `Uuid::now_v7` / `Uuid::new_v7` — a v7 id reads the wall clock and
//!   the OS RNG; mint it from the clock and RNG ports.
//!
//! Paths inside macro invocations (`tracing::info!`, `json!`, `select!`)
//! are read from the macro's tokens, since syn leaves them unparsed.
//!
//! The scanned crates are not a hand list: they are every workspace crate
//! that `rekindle-node` and `rekindle-netsim` reach through their normal
//! (not dev or build) path dependencies, read from each `Cargo.toml`, so a
//! new dependency of the Host is scanned the day it is added.
//!
//! [`OUTSIDE_THE_SIM`] excludes, by path, the code the simulator replaces
//! through a port, with the reason; there are no inline markers. Test code
//! (`#[cfg(test)]` modules, `#[test]`/`#[tokio::test]` functions, `tests/`
//! and `*_tests.rs` files) is not scanned.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use syn::visit::Visit;

use crate::walk_source_files;

/// The crates whose dependency closure runs in the simulator.
const ROOTS: &[&str] = &["rekindle-node", "rekindle-netsim"];

/// `(path prefix, why the simulator never runs it)`.
const OUTSIDE_THE_SIM: &[(&str, &str)] = &[
    (
        "crates/rekindle-utils/src/time.rs",
        "SystemClock, the production adapter behind the Clock port; the netsim injects SimClock",
    ),
    (
        "crates/rekindle-idle/src/platform/",
        "the OS idle readers behind the IdleSource port; the netsim injects SimIdle",
    ),
    (
        "crates/rekindle-ipc/src/client.rs",
        "the frontends' Unix-socket client; the netsim calls each Host in process, never over IPC",
    ),
    (
        "crates/rekindle-ipc/src/message.rs",
        "the IPC wire envelope's timestamp, built only by the excluded client and server",
    ),
    (
        "crates/rekindle-link-preview/src/lib.rs",
        "the fetched_at of an HTTPS preview fetch; the netsim makes no HTTP requests",
    ),
    (
        "crates/rekindle-ipc/src/server/",
        "the daemon's Unix-socket server and its connections; the netsim never binds a socket",
    ),
];

/// The resolved paths that read the wall clock, and the fix for each.
const FORBIDDEN: &[(&[&str], &str)] = &[
    (
        &["std", "time", "Instant", "now"],
        "use tokio::time::Instant",
    ),
    (&["std", "thread", "sleep"], "use tokio::time::sleep"),
];

/// The `rekindle_utils` wall-clock helpers (`SystemClock` reads through
/// them). Inside the sim a timestamp comes from the clock port, or from a
/// `now` the caller passes down to pure code.
const UTILS_WALL_CLOCK: &[&str] = &[
    "timestamp_ms",
    "timestamp_ms_i64",
    "timestamp_secs",
    "timestamp_secs_i64",
];

/// Suffixes forbidden whatever they resolve to (no async-clock twin exists).
const FORBIDDEN_SUFFIX: &[(&[&str], &str)] = &[
    (
        &["SystemTime", "now"],
        "take the timestamp from the clock port",
    ),
    (&["Utc", "now"], "take the timestamp from the clock port"),
    (&["Local", "now"], "take the timestamp from the clock port"),
    (
        &["Uuid", "now_v7"],
        "mint the id from the clock and RNG ports",
    ),
    (
        &["Uuid", "new_v7"],
        "mint the id from the clock and RNG ports",
    ),
];

/// Every `a::b::c` path in a macro's tokens, groups included.
fn token_paths(tokens: proc_macro2::TokenStream, out: &mut Vec<Vec<String>>) {
    use proc_macro2::{Spacing, TokenTree};
    let mut current: Vec<String> = Vec::new();
    // Whether the last tokens were `::`, so the next ident extends the path.
    let mut colons = 0;
    for tree in tokens {
        match tree {
            TokenTree::Ident(ident) => {
                if colons != 2 && !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                current.push(ident.to_string());
                colons = 0;
            }
            TokenTree::Punct(p) if p.as_char() == ':' && !current.is_empty() => {
                let joint_first = colons == 0 && p.spacing() == Spacing::Joint;
                colons = if joint_first || colons == 1 {
                    colons + 1
                } else {
                    0
                };
            }
            other => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                colons = 0;
                if let TokenTree::Group(group) = other {
                    token_paths(group.stream(), out);
                }
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
}

struct Finder<'a> {
    rel: &'a str,
    src: &'a str,
    cursor: usize,
    /// Imported name → full path, from the file's `use` items.
    uses: BTreeMap<String, Vec<String>>,
    hits: Vec<String>,
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

fn collect_uses(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            collect_uses(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            if n.ident == "self" {
                full.pop();
                if let Some(last) = full.last().cloned() {
                    out.insert(last, full);
                }
            } else {
                out.insert(n.ident.to_string(), full);
            }
        }
        syn::UseTree::Rename(r) => {
            let mut full = prefix.clone();
            full.push(r.ident.to_string());
            out.insert(r.rename.to_string(), full);
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_uses(t, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

struct UseCollector(BTreeMap<String, Vec<String>>);

impl<'ast> Visit<'ast> for UseCollector {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        collect_uses(&item.tree, &mut Vec::new(), &mut self.0);
    }
}

impl Finder<'_> {
    fn resolve(&self, segments: &[String]) -> Vec<String> {
        match segments.split_first() {
            Some((first, rest)) => match self.uses.get(first) {
                Some(path) => path.iter().chain(rest).cloned().collect(),
                None => segments.to_vec(),
            },
            None => Vec::new(),
        }
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
        self.hits
            .push(format!("{}:{line}  {needle} — {fix}", self.rel));
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !item.attrs.iter().any(is_test_attr) {
            syn::visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !item.attrs.iter().any(is_test_attr) {
            syn::visit::visit_item_fn(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !item.attrs.iter().any(is_test_attr) {
            syn::visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
        let segments: Vec<String> = expr
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        self.check_path(&segments);
        syn::visit::visit_expr_path(self, expr);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let mut paths = Vec::new();
        token_paths(mac.tokens.clone(), &mut paths);
        for segments in &paths {
            self.check_path(segments);
        }
        syn::visit::visit_macro(self, mac);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if !matches!(item.vis, syn::Visibility::Inherited) {
            let mut uses = BTreeMap::new();
            collect_uses(&item.tree, &mut Vec::new(), &mut uses);
            for full in uses.values() {
                if is_utils_helper(full) {
                    let needle = full.last().cloned().unwrap_or_default();
                    self.hit(&needle, "do not re-export a wall-clock helper");
                }
            }
        }
    }
}

fn is_utils_helper(full: &[String]) -> bool {
    full.first().is_some_and(|c| c == "rekindle_utils")
        && full
            .last()
            .is_some_and(|f| UTILS_WALL_CLOCK.contains(&f.as_str()))
}

impl Finder<'_> {
    fn check_path(&mut self, segments: &[String]) {
        let full = self.resolve(segments);
        let ends = |s: &[&str]| full.len() >= s.len() && full[full.len() - s.len()..] == *s;
        let utils_helper = is_utils_helper(&full);
        let fix = FORBIDDEN
            .iter()
            .find(|(p, _)| full == *p)
            .or_else(|| FORBIDDEN_SUFFIX.iter().find(|(s, _)| ends(s)))
            .map(|(_, fix)| *fix)
            .or_else(|| utils_helper.then_some("take the timestamp from the clock port"));
        if let Some(fix) = fix {
            self.hit(&segments.join("::"), fix);
        }
    }
}

/// The wall-clock reads in `src`.
fn scan(rel: &str, src: &str) -> Vec<String> {
    // A file that does not parse is cargo's problem to report.
    let Ok(parsed) = syn::parse_file(src) else {
        return Vec::new();
    };
    let mut uses = UseCollector(BTreeMap::new());
    uses.visit_file(&parsed);
    let mut finder = Finder {
        rel,
        src,
        cursor: 0,
        uses: uses.0,
        hits: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.hits
}

/// The workspace crates `name` depends on through normal path dependencies.
fn path_deps(root: &Path, name: &str) -> Result<Vec<String>> {
    let manifest = root.join("crates").join(name).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("reading {}", manifest.display()))?;
    let doc: toml_edit::DocumentMut = text.parse()?;
    let mut tables = vec![doc.get("dependencies")];
    if let Some(targets) = doc.get("target").and_then(|t| t.as_table_like()) {
        tables.extend(targets.iter().map(|(_, t)| t.get("dependencies")));
    }
    let mut deps = Vec::new();
    for table in tables
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_table_like())
    {
        for (dep, spec) in table.iter() {
            let path = spec.get("path").and_then(|p| p.as_str());
            if let Some(dir) = path.and_then(|p| p.strip_prefix("../")) {
                deps.push(dir.to_owned());
            } else if path.is_some() {
                deps.push(dep.to_owned());
            }
        }
    }
    Ok(deps)
}

/// Every workspace crate the simulator runs.
fn sim_crates(root: &Path) -> Result<BTreeSet<String>> {
    let mut seen = BTreeSet::new();
    let mut todo: Vec<String> = ROOTS.iter().map(|s| (*s).to_owned()).collect();
    while let Some(name) = todo.pop() {
        if seen.insert(name.clone()) {
            todo.extend(path_deps(root, &name)?);
        }
    }
    Ok(seen)
}

fn is_test_file(rel: &str) -> bool {
    rel.contains("/tests/") || rel.ends_with("_tests.rs") || rel.ends_with("/tests.rs")
}

pub(crate) fn check_wall_clock(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for name in sim_crates(root)? {
        for path in walk_source_files(&root.join("crates").join(&name).join("src"), &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if is_test_file(&rel) || OUTSIDE_THE_SIM.iter().any(|(p, _)| rel.starts_with(p)) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            hits.extend(scan(&rel, &src));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        eprintln!("    {h}");
    }
    Err(anyhow!(
        "{} wall-clock read(s) in code the netsim runs (plan E2.B)",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_std_instants_through_imports_and_renames() {
        let src = "use std::time::{Duration, Instant};
                   use std::time::Instant as Wall;
                   use std::time;
                   fn a() { Instant::now(); Wall::now(); time::Instant::now(); std::time::Instant::now(); }";
        let hits = scan("crates/x/src/a.rs", src);
        assert_eq!(hits.len(), 4, "{hits:?}");
        assert!(hits[0].contains("a.rs:4"), "{hits:?}");
    }

    #[test]
    fn passes_tokio_instants_and_flags_the_rest() {
        let src = "use tokio::time::Instant;
                   fn a() { Instant::now(); tokio::time::Instant::now().into_std(); }
                   fn b() { std::time::SystemTime::now(); chrono::Utc::now(); Local::now();
                            std::thread::sleep(d); tokio::time::sleep(d); }";
        let hits = scan("crates/x/src/a.rs", src);
        assert_eq!(hits.len(), 4, "{hits:?}");
        assert!(hits.iter().all(|h| !h.contains("  tokio")), "{hits:?}");
    }

    #[test]
    fn flags_the_utils_wall_clock_helpers() {
        let src = "use rekindle_utils::timestamp_ms;
                   use rekindle_utils::time;
                   fn a() { timestamp_ms(); rekindle_utils::timestamp_secs(); time::timestamp_ms_i64();
                            clock.now_ms(); }";
        let hits = scan("crates/x/src/a.rs", src);
        assert_eq!(hits.len(), 3, "{hits:?}");
    }

    #[test]
    fn flags_v7_uuids() {
        let src = "use uuid::Uuid;
                   fn a() { Uuid::now_v7(); uuid::Uuid::new_v7(ts); Uuid::new_v4(); }";
        let hits = scan("crates/x/src/a.rs", src);
        assert_eq!(hits.len(), 2, "{hits:?}");
    }

    #[test]
    fn flags_re_exports_of_the_helpers_under_any_name() {
        let src = "pub use rekindle_utils::{timestamp_ms, timestamp_secs as now_secs};
                   pub(crate) use rekindle_utils::timestamp_ms_i64;
                   use rekindle_utils::Clock;
                   pub use rekindle_utils::Clock as C;";
        let hits = scan("crates/x/src/lib.rs", src);
        assert_eq!(hits.len(), 3, "{hits:?}");
    }

    #[test]
    fn flags_reads_inside_macro_bodies() {
        let src = "use rekindle_utils::timestamp_ms;
                   use std::time::Instant;
                   fn a() {
                       tracing::info!(at = timestamp_ms(), \"x\");
                       let v = serde_json::json!({ \"t\": std::time::SystemTime::now() });
                       tokio::select! { _ = f() => { Instant::now(); } }
                       format!(\"{}\", tokio::time::Instant::now().elapsed().as_secs());
                   }";
        let hits = scan("crates/x/src/a.rs", src);
        assert_eq!(hits.len(), 3, "{hits:?}");
    }

    #[test]
    fn ignores_test_code() {
        let src = "use std::time::Instant;
                   #[cfg(test)] mod tests { fn t() { std::time::Instant::now(); } }
                   #[test] fn t2() { Instant::now(); }";
        assert!(scan("crates/x/src/a.rs", src).is_empty());
    }
}
