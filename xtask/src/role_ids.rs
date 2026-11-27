//! `check-role-ids`: role ids are written only by the governance projection
//! (plan E3.2, D15; review RV7 M1).
//!
//! A member's roles, and ours, are the merged `GovernanceState`'s role
//! assignments. `projection.rs` lays ours onto the community state
//! (`project_my_roles`) and derives every member's (`member_role_ids`); any
//! other copy is a cache that a later `RoleUnassignment` does not clear.
//! This gate rejects, in every `.rs` under `crates/` and `src-tauri/src`
//! outside the allowlist:
//! - an assignment to a field named `role_ids`, `my_role_ids` or
//!   `member_roles`, or a mutating method call on one (`push`, `retain`,
//!   `insert`, `extend`, `clone_from`, …);
//! - a definition of, or call to, a role-cache writer (`set_role_ids`,
//!   `set_my_role_ids`, `set_member_roles`, `apply_role_assignment`,
//!   `apply_role_unassignment`, `persist_member_roles`, `add_role`,
//!   `remove_role`, `remove_role_everywhere`);
//! - a string literal naming a `role_ids` column (SQL);
//!
//! and any `role_ids` column in a `.sql` file. Wire and event types in
//! `rekindle-codec` and `rekindle-types` carry role ids in ops and events;
//! they are not caches and are allowed.

use std::path::Path;

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::syn_scan::visit;

use crate::walk_source_files;

/// Paths (relative to the workspace root) whose role-id writes are allowed.
const ALLOWED: &[&str] = &[
    "crates/rekindle-node/src/host/adapters/governance/projection.rs",
    "crates/rekindle-codec/",
    "crates/rekindle-types/",
];

/// Fields that hold role ids.
const ROLE_FIELDS: &[&str] = &["role_ids", "my_role_ids", "member_roles"];

/// Methods that change a collection in place.
const MUTATORS: &[&str] = &[
    "push",
    "retain",
    "insert",
    "extend",
    "clone_from",
    "clear",
    "remove",
    "append",
    "sort",
    "sort_unstable",
    "dedup",
    "truncate",
];

/// Role-cache writers that must not come back.
const WRITERS: &[&str] = &[
    "set_role_ids",
    "set_my_role_ids",
    "set_member_roles",
    "apply_role_assignment",
    "apply_role_unassignment",
    "persist_member_roles",
    "add_role",
    "remove_role",
    "remove_role_everywhere",
];

struct Finder<'a> {
    rel: &'a str,
    hits: Vec<String>,
}

fn role_field(expr: &syn::Expr) -> Option<String> {
    let syn::Expr::Field(field) = expr else {
        return None;
    };
    let syn::Member::Named(name) = &field.member else {
        return None;
    };
    let name = name.to_string();
    ROLE_FIELDS.contains(&name.as_str()).then_some(name)
}

impl Finder<'_> {
    fn hit(&mut self, what: &str) {
        self.hits.push(format!("{}  {what}", self.rel));
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    // Doc comments and other attributes are prose, not writes.
    fn visit_attribute(&mut self, _attr: &'ast syn::Attribute) {}

    fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
        if let Some(field) = role_field(&assign.left) {
            self.hit(&format!("assignment to `.{field}`"));
        }
        syn::visit::visit_expr_assign(self, assign);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if MUTATORS.contains(&method.as_str()) {
            if let Some(field) = role_field(&call.receiver) {
                self.hit(&format!("`.{field}.{method}(…)`"));
            }
        }
        if WRITERS.contains(&method.as_str()) {
            self.hit(&format!("role-cache writer `.{method}(…)`"));
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(last) = path.segments.last() {
            let name = last.ident.to_string();
            if WRITERS.contains(&name.as_str()) {
                self.hit(&format!("role-cache writer `{name}`"));
            }
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_signature(&mut self, sig: &'ast syn::Signature) {
        let name = sig.ident.to_string();
        if WRITERS.contains(&name.as_str()) {
            self.hit(&format!("role-cache writer `fn {name}`"));
        }
        syn::visit::visit_signature(self, sig);
    }

    fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
        if names_role_column(&lit.value()) {
            self.hit("SQL naming a `role_ids` column");
        }
        syn::visit::visit_lit_str(self, lit);
    }
}

/// Whether `text` names a `role_ids`/`my_role_ids` column as a word.
fn names_role_column(text: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let starts_word = |prefix: &str| !prefix.chars().next_back().is_some_and(word);
    text.match_indices("role_ids").any(|(at, m)| {
        let prefix = &text[..at];
        let whole = starts_word(prefix) || prefix.strip_suffix("my_").is_some_and(&starts_word);
        whole && !text[at + m.len()..].chars().next().is_some_and(word)
    })
}

fn scan(rel: &str, src: &str) -> Vec<String> {
    visit(
        src,
        Finder {
            rel,
            hits: Vec::new(),
        },
    )
    .map_or_else(Vec::new, |f| f.hits)
}

fn scan_sql(rel: &str, src: &str) -> Vec<String> {
    src.lines()
        .enumerate()
        .filter(|(_, line)| {
            let code = line.split("--").next().unwrap_or_default();
            names_role_column(code)
        })
        .map(|(i, _)| format!("{rel}:{}  a `role_ids` column", i + 1))
        .collect()
}

pub(crate) fn check_role_ids(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for dir in ["crates", "src-tauri/src"] {
        for path in walk_source_files(&root.join(dir), &["rs", "sql"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if ALLOWED.iter().any(|allowed| rel.starts_with(allowed)) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("sql"))
            {
                hits.extend(scan_sql(&rel, &src));
            } else {
                hits.extend(scan(&rel, &src));
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
        "{} role-id write(s) outside the governance projection. Read roles from \
         the merged role assignments (`projection::member_role_ids`, \
         `CommunityStore::my_role_ids`); never cache them.",
        hits.len()
    ))
}

#[cfg(test)]
#[path = "role_ids_tests.rs"]
mod tests;
