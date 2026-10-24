//! `check-key-erasure`: a key-material row is deleted only through
//! `rekindle_db::erase::delete` (threat model I14; review RV5 M3).
//!
//! A WAL keeps a deleted row's page images until it is checkpointed and
//! truncated, so every delete of key material must mark its connection
//! erased, which `erase::delete` does. This gate rejects, in non-test code
//! under `crates/*/src` and `src-tauri/src`, a string literal holding
//! `DELETE FROM <table>` for a table in
//! `rekindle_db::erase::KEY_MATERIAL_TABLES` unless it is an argument of a
//! call to `erase::delete`. Test code is not scanned.

use std::path::Path;

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::walk_source_files;
use crate::wall_clock::{is_test_attr, is_test_file};

/// `rekindle_db::erase::KEY_MATERIAL_TABLES`, read from its source so the
/// two never disagree.
const ERASE_FILE: &str = "crates/rekindle-db/src/erase.rs";

/// The table names in `KEY_MATERIAL_TABLES` in `src` (the erase module).
fn key_tables(src: &str) -> Vec<String> {
    let Some(start) = src.find("KEY_MATERIAL_TABLES") else {
        return Vec::new();
    };
    let rest = &src[start..];
    let body = rest
        .find('[')
        .and_then(|_| rest.find("= [").map(|i| &rest[i + 3..]))
        .and_then(|r| r.find("];").map(|end| &r[..end]))
        .unwrap_or_default();
    body.split(',')
        .map(|t| t.trim().trim_matches('"').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn deletes_key_material(sql: &str, tables: &[String]) -> Option<String> {
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    tables
        .iter()
        .find(|t| {
            let needle = format!("DELETE FROM {t}");
            flat.match_indices(&needle).any(|(i, _)| {
                !flat[i + needle.len()..]
                    .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            })
        })
        .cloned()
}

fn is_erase_delete(func: &syn::Expr) -> bool {
    let syn::Expr::Path(p) = func else {
        return false;
    };
    let names: Vec<String> = p
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    names.ends_with(&["erase".to_string(), "delete".to_string()])
}

struct Finder<'a> {
    rel: &'a str,
    tables: &'a [String],
    hits: Vec<String>,
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if !m.attrs.iter().any(is_test_attr) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if !f.attrs.iter().any(is_test_attr) {
            syn::visit::visit_item_fn(self, f);
        }
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if is_erase_delete(&call.func) {
            // Its SQL is the erasing delete; its other arguments are not SQL.
            return;
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
        if let Some(table) = deletes_key_material(&lit.value(), self.tables) {
            self.hits.push(format!(
                "{}  DELETE FROM {table} outside erase::delete",
                self.rel
            ));
        }
    }
}

fn scan(rel: &str, src: &str, tables: &[String]) -> Vec<String> {
    let Ok(parsed) = syn::parse_file(src) else {
        return Vec::new();
    };
    let mut finder = Finder {
        rel,
        tables,
        hits: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.hits
}

pub(crate) fn check_key_erasure(root: &Path) -> Result<()> {
    let tables = key_tables(&std::fs::read_to_string(root.join(ERASE_FILE))?);
    if tables.is_empty() {
        return Err(anyhow!("{ERASE_FILE}: KEY_MATERIAL_TABLES not found"));
    }
    let mut dirs = vec![root.join("src-tauri/src")];
    for entry in std::fs::read_dir(root.join("crates"))? {
        dirs.push(entry?.path().join("src"));
    }
    let mut hits = Vec::new();
    for dir in dirs {
        for path in walk_source_files(&dir, &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if is_test_file(&rel) || rel == ERASE_FILE {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            hits.extend(scan(&rel, &src, &tables));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        eprintln!("    {h}");
    }
    Err(anyhow!(
        "{} key-material delete(s) outside rekindle_db::erase::delete (threat model I14)",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::{key_tables, scan};

    fn tables() -> Vec<String> {
        key_tables("pub const KEY_MATERIAL_TABLES: [&str; 2] = [\n \"ratchet_sessions\",\n \"signal_prekeys\",\n];")
    }

    #[test]
    fn a_key_delete_outside_the_helper_is_refused() {
        let raw = "fn f(c: &C) { c.execute(\"DELETE FROM ratchet_sessions WHERE x = 1\", []); }";
        assert_eq!(scan("crates/x/src/lib.rs", raw, &tables()).len(), 1);
        let split = "fn f(c: &C) { c.execute(\"DELETE FROM signal_prekeys \\\n WHERE x\", []); }";
        assert_eq!(scan("crates/x/src/lib.rs", split, &tables()).len(), 1);
    }

    #[test]
    fn the_helper_and_other_tables_pass() {
        let helper =
            "fn f(c: &C) { crate::erase::delete(c, \"DELETE FROM ratchet_sessions\", []); }";
        assert!(scan("crates/x/src/lib.rs", helper, &tables()).is_empty());
        let other = "fn f(c: &C) { c.execute(\"DELETE FROM ratchet_sessions_log\", []); }";
        assert!(scan("crates/x/src/lib.rs", other, &tables()).is_empty());
        let test = "#[cfg(test)] mod t { fn f() { x(\"DELETE FROM ratchet_sessions\"); } }";
        assert!(scan("crates/x/src/lib.rs", test, &tables()).is_empty());
    }
}
