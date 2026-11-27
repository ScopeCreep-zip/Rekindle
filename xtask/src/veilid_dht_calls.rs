//! `check-veilid-dht-calls`: every Veilid DHT and private-route call has
//! one owner (plan C7.11).
//!
//! - DHT record calls (`open_dht_record`, `get_dht_value`, …) run only in
//!   the record pool's Veilid backend, `rekindle-protocol/src/dht/backend.rs`
//!   (plan E2.B): the pool calls it through `rekindle_records::port::
//!   DhtBackend`, and holds each record's lease, writer and watch,
//!   serializes per subkey, and never aborts a call mid-commit (plans
//!   C7.3–C7.6).
//! - `import_remote_private_route` runs only in the process's one importer,
//!   `dht/route_imports.rs` (plan C7.2): a second importer's release drops
//!   the shared import for every sender.
//! - Our own private routes are allocated and released only by their
//!   owners: `own_routes.rs` (general and media, plan C7.9) and the relay
//!   feature's dedicated routes (`services/relay/offer.rs`, plan C7.9f). A
//!   remote route is never released: Veilid releases a dead one itself
//!   (`ping_validator.rs:610-628`, plan C7.9e).
//!
//! - `rekindle-transport` and `rekindle-records` name no `veilid_core` at
//!   all, in code, comments or manifest (plan C9.1): the transport runs on
//!   the Veilid-free ports of `rekindle_records::port`, which only
//!   `rekindle-protocol` implements over Veilid.
//!
//! Only crates that depend on `veilid-core` can make these calls, so only
//! they are scanned: a deps trait in a Veilid-free crate may reuse a method
//! name (`get_dht_value` on governance-runtime's overflow seam) without
//! being a Veilid call.

use std::path::{Path, PathBuf};

use anyhow::Result;
use syn::visit::Visit;

use crate::syn_scan::{verdict, visit, LineCursor};

use crate::walk::{read_files_under, SourcePath};

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
        owners: &["crates/rekindle-protocol/src/dht/backend.rs"],
        why: "DHT calls go through the record pool's backend (rekindle_protocol::dht::backend)",
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
        why:
            "own routes are allocated and released only by VeilidRouteAllocator or the relay offer",
    },
];

struct Finder<'a> {
    rel: &'a str,
    lines: LineCursor<'a>,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn hit(&mut self, method: &str, why: &str) {
        let needle = format!(".{method}(");
        let line = self.lines.line(&needle);
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

/// The crates that run on the ports and must name no Veilid type (plan
/// C9.1).
const VEILID_FREE: &[&str] = &[
    "rekindle-transport",
    "rekindle-records",
    "rekindle-record-pool",
    "rekindle-netsim",
];

/// Every mention of `veilid_core` (or a `veilid-core` dependency) in a
/// [`VEILID_FREE`] crate.
fn veilid_free_hits(root: &Path) -> Result<Vec<String>> {
    let mut hits = Vec::new();
    for name in VEILID_FREE {
        let dir = root.join("crates").join(name);
        let manifest = dir.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            let doc: toml_edit::DocumentMut = text.parse()?;
            for table in ["dependencies", "dev-dependencies"] {
                if doc.get(table).and_then(|t| t.get("veilid-core")).is_some() {
                    hits.push(format!(
                        "crates/{name}/Cargo.toml  [{table}] veilid-core — {name} runs on the ports (plan C9.1)"
                    ));
                }
            }
        }
        for (SourcePath { rel, .. }, src) in read_files_under(root, &dir.join("src"), &["rs"])? {
            hits.extend(names_veilid_core(&rel, &src));
        }
    }
    Ok(hits)
}

fn names_veilid_core(rel: &str, src: &str) -> Vec<String> {
    src.lines()
        .enumerate()
        .filter(|(_, line)| line.contains("veilid_core"))
        .map(|(i, _)| {
            format!(
                "{rel}:{}  veilid_core — this crate runs on the ports (plan C9.1)",
                i + 1
            )
        })
        .collect()
}

pub(crate) fn check_veilid_dht_calls(root: &Path) -> Result<()> {
    let mut hits = veilid_free_hits(root)?;
    for dir in veilid_linked_sources(root)? {
        for (SourcePath { rel, .. }, src) in read_files_under(root, &dir, &["rs"])? {
            hits.extend(scan(&rel, &src));
        }
    }
    verdict(&hits, "Veilid DHT/route call(s) outside their owner, or Veilid in a port crate (plans C7.11, C9.1).")
}

#[cfg(test)]
#[path = "veilid_dht_calls_tests.rs"]
mod tests;
