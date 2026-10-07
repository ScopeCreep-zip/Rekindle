//! `check-sqlite`: SQL lives in the storage crates (plan C5.7).
//!
//! `rekindle-db` holds the schema and the repositories, `rekindle-vault`
//! its own encrypted store, `rekindle-asql` the connection thread. A query
//! written anywhere else is a second copy of a repository that will drift
//! from it (the C5.3 hoist found three queries that had dropped the owner
//! scope). Two things are rejected outside the allowed crates:
//! - a dependency on `rusqlite` or `rekindle-asql`;
//! - a string literal shaped like SQL (`SELECT … FROM`, `INSERT … INTO`,
//!   `UPDATE … SET`, `DELETE FROM`, `CREATE TABLE|INDEX|TRIGGER|VIRTUAL`,
//!   `PRAGMA`), uppercase as every query in the tree is written, so prose
//!   does not match.

use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::{literals, walk_source_files};

/// The storage crates: SQL belongs here.
const OWNERS: &[&str] = &[
    "crates/rekindle-db",
    "crates/rekindle-vault",
    "crates/rekindle-asql",
];

/// Crates that still hold their own SQL, each removed by the plan step
/// named. The list only shrinks.
const SHRINKING: &[(&str, &str)] = &[
    (
        "crates/rekindle-dm",
        "E2.3 deletes the rekindle-dm 1:1 path",
    ),
    (
        "crates/rekindle-analytics",
        "E5 moves its aggregate queries into rekindle-db",
    ),
    ("crates/rekindle-e2e-server", "F1 makes it a bus bridge"),
    (
        "src-tauri",
        "the E phase moves each domain's SQL with the domain",
    ),
];

const DRIVERS: &[&str] = &["rusqlite", "rekindle-asql"];

fn allowed(rel: &str) -> bool {
    OWNERS
        .iter()
        .chain(SHRINKING.iter().map(|(dir, _)| dir))
        .any(|dir| rel == *dir || rel.starts_with(&format!("{dir}/")))
}

/// Does `s` read as a SQL statement?
pub(crate) fn is_sql_shaped(s: &str) -> bool {
    let words: Vec<&str> = s.split_whitespace().collect();
    let has = |w: &str| words.contains(&w);
    let follows = |first: &str, then: &[&str]| {
        words
            .windows(2)
            .any(|pair| pair[0] == first && then.contains(&pair[1]))
    };
    (has("SELECT") && has("FROM"))
        || (has("INSERT") && has("INTO"))
        || words.windows(3).any(|w| w[0] == "UPDATE" && w[2] == "SET")
        || follows("DELETE", &["FROM"])
        || follows(
            "CREATE",
            &["TABLE", "INDEX", "TRIGGER", "VIRTUAL", "UNIQUE"],
        )
        || words.first() == Some(&"PRAGMA")
}

/// Packages outside the storage crates that depend on a SQLite driver.
fn driver_dependencies(root: &Path) -> Result<Vec<String>> {
    let mut hits = Vec::new();
    let manifests = walk_source_files(&root.join("crates"), &["toml"])?
        .into_iter()
        .chain(std::iter::once(root.join("src-tauri/Cargo.toml")))
        .filter(|p| p.file_name().is_some_and(|n| n == "Cargo.toml"));
    for manifest in manifests {
        let rel_dir = manifest
            .parent()
            .and_then(|d| d.strip_prefix(root).ok())
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        if allowed(&rel_dir) {
            continue;
        }
        let text = std::fs::read_to_string(&manifest)
            .with_context(|| format!("reading {}", manifest.display()))?;
        let doc: toml_edit::DocumentMut = text
            .parse()
            .with_context(|| format!("parsing {}", manifest.display()))?;
        for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let Some(deps) = doc.get(table).and_then(toml_edit::Item::as_table_like) else {
                continue;
            };
            for (name, spec) in deps.iter() {
                let package = spec
                    .get("package")
                    .and_then(toml_edit::Item::as_str)
                    .unwrap_or(name);
                if DRIVERS.contains(&package) {
                    hits.push(format!("{rel_dir}/Cargo.toml  [{table}] {name}"));
                }
            }
        }
    }
    Ok(hits)
}

/// SQL-shaped literals in source outside the storage crates.
fn sql_literals(root: &Path) -> Result<Vec<String>> {
    let mut hits = Vec::new();
    for path in walk_source_files(&root.join("crates"), &["rs"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if allowed(&rel) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        // A file that does not parse is a compiler problem, not this gate's.
        let Ok(parsed) = syn::parse_file(&src) else {
            continue;
        };
        literals::for_each(&parsed, |value, token| {
            if is_sql_shaped(value) {
                let line = literals::line_of(&src, token);
                hits.push(format!(
                    "{rel}:{line}  {}",
                    value.split_whitespace().collect::<Vec<_>>().join(" ")
                ));
            }
        });
    }
    Ok(hits)
}

pub(crate) fn check_sqlite(root: &Path) -> Result<()> {
    let mut hits = driver_dependencies(root)?;
    hits.extend(sql_literals(root)?);
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} SQLite use(s) outside the storage crates.\n\
         Add the query to the table's repository in rekindle-db (crates/rekindle-db/src/repo/) \
         and call it.",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::is_sql_shaped;

    #[test]
    fn sql_shapes() {
        for s in [
            "SELECT role_ids FROM community_members WHERE owner_key = ?1",
            "INSERT OR IGNORE INTO friends (owner_key) VALUES (?1)",
            "UPDATE communities SET name = ?1",
            "DELETE FROM identity WHERE public_key = ?1",
            "CREATE TABLE IF NOT EXISTS t (a TEXT)",
            "CREATE VIRTUAL TABLE messages_fts USING fts5(body)",
            "PRAGMA journal_mode=WAL;",
        ] {
            assert!(is_sql_shaped(s), "{s}");
        }
        for s in [
            "select a friend from the list",
            "failed to update the friend list",
            "Delete from your device?",
            "SELECT",
            "the INSERT key",
        ] {
            assert!(!is_sql_shaped(s), "{s}");
        }
    }
}
