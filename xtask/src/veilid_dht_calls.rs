//! `check-veilid-dht-calls`: every Veilid DHT and private-route call has
//! one owner (plan C7.11).
//!
//! - DHT record calls (`open_dht_record`, `get_dht_value`, …) run only in
//!   the record pool, `rekindle-protocol/src/dht/pool/`: it holds each
//!   record's lease, writer and watch, serializes per subkey, and never
//!   aborts a call mid-commit (plans C7.3–C7.6).
//! - `import_remote_private_route` runs only in the process's one importer,
//!   `dht/route_imports.rs` (plan C7.2): a second importer's release drops
//!   the shared import for every sender.
//! - Our own private routes are allocated and released only by their
//!   owners: `own_routes.rs` (general and media, plan C7.9) and the relay
//!   feature's dedicated routes (`services/relay/offer.rs`, plan C7.9f). A
//!   remote route is never released: Veilid releases a dead one itself
//!   (`ping_validator.rs:610-628`, plan C7.9e).
//!
//! Only crates that depend on `veilid-core` can make these calls, so only
//! they are scanned: a deps trait in a Veilid-free crate may reuse a method
//! name (`get_dht_value` on governance-runtime's overflow seam) without
//! being a Veilid call.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::walk_source_files;

/// A Veilid method and the source paths (relative to the workspace root,
/// prefix match) allowed to call it.
struct Rule {
    methods: &'static [&'static str],
    owners: &'static [&'static str],
    why: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        methods: &[
            "create_dht_record",
            "open_dht_record",
            "close_dht_record",
            "delete_dht_record",
            "get_dht_value",
            "set_dht_value",
            "inspect_dht_record",
            "watch_dht_values",
            "cancel_dht_watch",
            "transact_dht_records",
        ],
        owners: &["crates/rekindle-protocol/src/dht/pool/"],
        why: "DHT calls go through the record pool (rekindle_protocol::dht::pool)",
    },
    Rule {
        methods: &["import_remote_private_route"],
        owners: &["crates/rekindle-protocol/src/dht/route_imports.rs"],
        why: "peer routes are imported only by RouteImports",
    },
    Rule {
        methods: &[
            "new_private_route",
            "new_custom_private_route",
            "release_private_route",
        ],
        owners: &[
            "crates/rekindle-protocol/src/own_routes.rs",
            "src-tauri/src/services/relay/offer.rs",
        ],
        why: "own routes are allocated and released only by OwnRoutes or the relay offer",
    },
];

struct Finder<'a> {
    rel: &'a str,
    src: &'a str,
    /// Byte offset past the previous hit's text, so repeated occurrences
    /// report their own lines (no `span-locations` on proc-macro2).
    cursor: usize,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn hit(&mut self, method: &str, why: &str) {
        let needle = format!(".{method}(");
        let line = match self.src[self.cursor..].find(&needle) {
            Some(at) => {
                let abs = self.cursor + at;
                self.cursor = abs + needle.len();
                self.src[..abs].matches('\n').count() + 1
            }
            None => 0,
        };
        self.hits
            .push(format!("{}:{line}  .{method}(…) — {why}", self.rel));
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        for rule in RULES {
            if rule.methods.contains(&name.as_str())
                && !rule.owners.iter().any(|owner| self.rel.starts_with(owner))
            {
                self.hit(&name, rule.why);
            }
        }
        syn::visit::visit_expr_method_call(self, call);
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

/// The source directories of every workspace package that depends on
/// `veilid-core`.
fn veilid_linked_sources(root: &Path) -> Result<Vec<PathBuf>> {
    let mut manifests: Vec<PathBuf> = std::fs::read_dir(root.join("crates"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("Cargo.toml"))
        .collect();
    manifests.push(root.join("src-tauri/Cargo.toml"));
    let mut dirs = Vec::new();
    for manifest in manifests {
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let doc: toml_edit::DocumentMut = text.parse()?;
        let depends = ["dependencies", "dev-dependencies"]
            .iter()
            .any(|table| doc.get(table).and_then(|t| t.get("veilid-core")).is_some());
        if depends {
            if let Some(dir) = manifest.parent() {
                dirs.push(dir.join("src"));
            }
        }
    }
    dirs.sort();
    Ok(dirs)
}

pub(crate) fn check_veilid_dht_calls(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for dir in veilid_linked_sources(root)? {
        for path in walk_source_files(&dir, &["rs"])? {
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
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} Veilid DHT/route call(s) outside their owner (plan C7.11).",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::scan;

    #[test]
    fn flags_calls_outside_their_owner() {
        let src = r"
            async fn f(rc: veilid_core::RoutingContext, api: veilid_core::VeilidAPI) {
                let _ = rc.get_dht_value(k, 0, false).await;
                let _ = api.import_remote_private_route(blob);
                let _ = api.new_private_route().await;
            }
        ";
        let hits = scan("src-tauri/src/services/x.rs", src);
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert!(hits[0].starts_with("src-tauri/src/services/x.rs:3"));
    }

    #[test]
    fn allows_each_owner_its_own_calls() {
        let pool = "fn f() { rc.set_dht_value(k, 0, v, None); }";
        assert!(scan("crates/rekindle-protocol/src/dht/pool/io.rs", pool).is_empty());
        let imports = "fn f() { api.import_remote_private_route(b); }";
        assert!(scan("crates/rekindle-protocol/src/dht/route_imports.rs", imports).is_empty());
        let own = "fn f() { api.release_private_route(id); }";
        assert!(scan("crates/rekindle-protocol/src/own_routes.rs", own).is_empty());
        assert!(scan("src-tauri/src/services/relay/offer.rs", own).is_empty());
    }

    #[test]
    fn an_owner_of_one_rule_is_not_an_owner_of_another() {
        let src = "fn f() { api.release_private_route(id); }";
        assert_eq!(
            scan("crates/rekindle-protocol/src/dht/route_imports.rs", src).len(),
            1,
            "the importer never releases"
        );
    }
}
