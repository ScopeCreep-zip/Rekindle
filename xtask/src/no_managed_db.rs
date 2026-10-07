//! `check-no-managed-db`: the database is reached through `AppState.db`,
//! never through Tauri managed state (plan C6).
//!
//! `DbHandle::current()` is what makes "no identity is loaded" an explicit
//! error; a database put in Tauri's managed state is always there, so any
//! command or service that read it would keep writing after logout (and,
//! from plan D1, into whichever identity's file it was handed at boot).
//! This gate rejects, in `src-tauri` and `crates/rekindle-e2e-server`:
//! - a `State<'_, Db>` (or `Db`) type;
//! - `state::<Db>()` / `try_state::<Db>()`;
//! - a `.manage(…)` call whose argument names a database (`pool`, `db`).

use std::path::Path;

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::{literals, walk_source_files};

const DB_TYPES: &[&str] = &["Db", "Db"];

fn names_db(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(p)
        if p.path.segments.last().is_some_and(|s| DB_TYPES.iter().any(|d| s.ident == d)))
}

struct Finder<'a> {
    rel: &'a str,
    src: &'a str,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn hit(&mut self, needle: &str, what: &str) {
        let line = literals::line_of(self.src, needle);
        self.hits.push(format!("{}:{line}  {what}", self.rel));
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_path_segment(&mut self, seg: &'ast syn::PathSegment) {
        if seg.ident == "State" {
            if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                let db = args
                    .args
                    .iter()
                    .any(|a| matches!(a, syn::GenericArgument::Type(t) if names_db(t)));
                if db {
                    self.hit(
                        "State<",
                        "the database as Tauri `State` — use `state.db.current()`",
                    );
                }
            }
        }
        syn::visit::visit_path_segment(self, seg);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        let turbofish_db = call.turbofish.as_ref().is_some_and(|t| {
            t.args
                .iter()
                .any(|a| matches!(a, syn::GenericArgument::Type(ty) if names_db(ty)))
        });
        if (name == "state" || name == "try_state") && turbofish_db {
            self.hit(
                &format!("{name}::<"),
                "the database from Tauri managed state",
            );
        }
        if name == "manage" {
            let arg = call
                .args
                .first()
                .map(|a| quote::quote!(#a).to_string())
                .unwrap_or_default();
            if arg.contains("pool") || arg == "db" || arg.ends_with(". db") {
                self.hit(
                    ".manage(",
                    "the database put in Tauri managed state — use `state.db.set`",
                );
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_path(&mut self, e: &'ast syn::ExprPath) {
        // `tauri::Manager::try_state::<Db>(&app)` — a function-call form.
        let segs = &e.path.segments;
        if let Some(last) = segs.last() {
            if last.ident == "state" || last.ident == "try_state" {
                if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
                    if args
                        .args
                        .iter()
                        .any(|a| matches!(a, syn::GenericArgument::Type(t) if names_db(t)))
                    {
                        self.hit(
                            &format!("{}::<", last.ident),
                            "the database from Tauri managed state",
                        );
                    }
                }
            }
        }
        syn::visit::visit_expr_path(self, e);
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
        hits: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.hits
}

pub(crate) fn check_no_managed_db(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for dir in [
        "src-tauri/src",
        "src-tauri/tests",
        "crates/rekindle-e2e-server/src",
    ] {
        for path in walk_source_files(&root.join(dir), &["rs"])? {
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
        "{} use(s) of the database through Tauri managed state. Reach it with \
         `state.db.current()` (rekindle_db::DbHandle).",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::scan;

    #[test]
    fn flags_managed_database_access() {
        let bad = r"
            fn a(pool: tauri::State<'_, rekindle_db::Db>) {}
            fn b(app: &tauri::AppHandle) {
                let p = app.try_state::<Db>();
                let q = tauri::Manager::try_state::<crate::db::Db>(app);
                app.manage(pool);
            }
        ";
        assert_eq!(scan("x.rs", bad).len(), 4);
    }

    #[test]
    fn allows_the_handle_and_other_state() {
        let good = r"
            fn a(state: tauri::State<'_, SharedState>, pool: &rekindle_db::Db) {
                let db = state.db.current();
                app.manage(root);
                let k: tauri::State<'_, KeystoreHandle> = app.state();
            }
        ";
        assert!(scan("x.rs", good).is_empty());
    }
}
