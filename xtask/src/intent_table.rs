//! `check-intent-table` (rule B23, plan U1.2): every Tauri command
//! registered in `src-tauri/src/invoke.rs` and every `IpcRequest` variant
//! is classified in `rekindle_types::intent::COMMAND_TABLE`, and the table
//! names nothing that does not exist. A new command does not ship until it
//! is an intent of some class or listed as not one, so no frontend can
//! carry an unclassified action past the gate.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

const INVOKE: &str = "src-tauri/src/invoke.rs";
const IPC_PROTOCOL: &str = "crates/rekindle-ipc/src/protocol/mod.rs";
const TABLE: &str = "crates/rekindle-types/src/intent/table.rs";

/// The command names in `invoke.rs`'s `generate_handler!` list.
fn tauri_commands(src: &str) -> Vec<String> {
    src.split("commands::")
        .skip(1)
        .filter_map(|rest| {
            let path: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            path.rsplit("::").next().map(str::to_string)
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// The variant names of `pub enum IpcRequest`.
fn ipc_requests(src: &str) -> Result<Vec<String>> {
    let file = syn::parse_file(src).context("parsing the IPC protocol")?;
    file.items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == "IpcRequest" => {
                Some(e.variants.iter().map(|v| v.ident.to_string()).collect())
            }
            _ => None,
        })
        .ok_or_else(|| anyhow!("no `enum IpcRequest` in {IPC_PROTOCOL}"))
}

/// The first element of every `(name, kind)` row of `COMMAND_TABLE`.
fn table_names(src: &str) -> Result<Vec<String>> {
    let file = syn::parse_file(src).context("parsing the intent table")?;
    let table = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Const(c) if c.ident == "COMMAND_TABLE" => Some(c),
            _ => None,
        })
        .ok_or_else(|| anyhow!("no `const COMMAND_TABLE` in {TABLE}"))?;
    let syn::Expr::Reference(reference) = &*table.expr else {
        return Err(anyhow!("COMMAND_TABLE is not `&[…]`"));
    };
    let syn::Expr::Array(rows) = &*reference.expr else {
        return Err(anyhow!("COMMAND_TABLE is not `&[…]`"));
    };
    rows.elems
        .iter()
        .map(|row| match row {
            syn::Expr::Tuple(t) => match t.elems.first() {
                Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                })) => Ok(s.value()),
                _ => Err(anyhow!("a COMMAND_TABLE row does not start with a name")),
            },
            _ => Err(anyhow!("a COMMAND_TABLE row is not a tuple")),
        })
        .collect()
}

/// Every mismatch between the code's names and the table's.
fn compare(commands: &[String], requests: &[String], table: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for name in table {
        if !seen.insert(name.as_str()) {
            problems.push(format!("`{name}` is listed twice in {TABLE}"));
        }
    }
    let code: BTreeSet<&str> = commands
        .iter()
        .chain(requests)
        .map(String::as_str)
        .collect();
    for name in &code {
        if !seen.contains(name) {
            problems.push(format!(
                "`{name}` is not classified in {TABLE} (an IntentKind, or None for a read)"
            ));
        }
    }
    for name in &seen {
        if !code.contains(name) {
            problems.push(format!(
                "{TABLE} lists `{name}`, which is neither a registered command nor an IpcRequest"
            ));
        }
    }
    problems
}

pub(crate) fn check_intent_table(root: &Path) -> Result<()> {
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel)).with_context(|| format!("reading {rel}"))
    };
    let problems = compare(
        &tauri_commands(&read(INVOKE)?),
        &ipc_requests(&read(IPC_PROTOCOL)?)?,
        &table_names(&read(TABLE)?)?,
    );
    if problems.is_empty() {
        return Ok(());
    }
    for p in &problems {
        eprintln!("  ✗ {p}");
    }
    Err(anyhow!(
        "{} intent-table problem(s); classify per realignment/r9-intents.md §7.2",
        problems.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_three_sources_parse() {
        let invoke = "tauri::generate_handler![\n commands::auth::login,\n #[cfg(debug_assertions)]\n commands::auth::pqxdh_bundle_info,\n]";
        assert_eq!(
            tauri_commands(invoke),
            names(&["login", "pqxdh_bundle_info"])
        );
        let ipc = "pub enum IpcRequest { Unlock { passphrase: String }, Lock }";
        assert_eq!(ipc_requests(ipc).unwrap(), names(&["Unlock", "Lock"]));
        let table = r#"pub const COMMAND_TABLE: &[(&str, Option<IntentKind>)] = &[("login", Some(IntentKind::Login)), ("Lock", None)];"#;
        assert_eq!(table_names(table).unwrap(), names(&["login", "Lock"]));
    }

    /// Mutations: an unclassified command, an unclassified request, a
    /// stale row and a duplicate row each fail.
    #[test]
    fn every_mismatch_is_reported() {
        let ok = compare(
            &names(&["login"]),
            &names(&["Lock"]),
            &names(&["login", "Lock"]),
        );
        assert!(ok.is_empty());
        let new_command = compare(
            &names(&["login", "kick"]),
            &names(&["Lock"]),
            &names(&["login", "Lock"]),
        );
        assert_eq!(new_command.len(), 1);
        assert!(new_command[0].contains("`kick` is not classified"));
        let new_request = compare(
            &names(&["login"]),
            &names(&["Lock", "Ban"]),
            &names(&["login", "Lock"]),
        );
        assert!(new_request[0].contains("`Ban`"));
        let stale = compare(
            &names(&["login"]),
            &names(&["Lock"]),
            &names(&["login", "Lock", "gone"]),
        );
        assert!(stale[0].contains("`gone`"));
        let twice = compare(
            &names(&["login"]),
            &names(&["Lock"]),
            &names(&["login", "Lock", "login"]),
        );
        assert!(twice[0].contains("listed twice"));
    }
}
