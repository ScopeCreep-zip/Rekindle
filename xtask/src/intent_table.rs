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
const IPC_COMMUNITY: &str = "crates/rekindle-ipc/src/protocol/community.rs";
const TABLE: &str = "crates/rekindle-types/src/intent/table";

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

/// The variant names of `pub enum {name}` in `src`.
fn enum_variants(src: &str, name: &str, path: &str) -> Result<Vec<String>> {
    let file = syn::parse_file(src).with_context(|| format!("parsing {path}"))?;
    file.items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == name => {
                Some(e.variants.iter().map(|v| v.ident.to_string()).collect())
            }
            _ => None,
        })
        .ok_or_else(|| anyhow!("no `enum {name}` in {path}"))
}

/// The leaf names of `pub enum IpcRequest`: its `Community` variant stands
/// for every `CommunityRequest` variant, which `IpcRequest::name` answers.
fn ipc_requests(protocol: &str, community: &str) -> Result<Vec<String>> {
    let top = enum_variants(protocol, "IpcRequest", IPC_PROTOCOL)?;
    if !top.iter().any(|v| v == "Community") {
        return Err(anyhow!("no `IpcRequest::Community` in {IPC_PROTOCOL}"));
    }
    let nested = enum_variants(community, "CommunityRequest", IPC_COMMUNITY)?;
    Ok(top
        .into_iter()
        .filter(|v| v != "Community")
        .chain(nested)
        .collect())
}

/// The first element of every `(name, kind)` row of one part of
/// `COMMAND_TABLE`: each `const …_ROWS: &[…] = &[…]` in a file of `intent/table/`.
fn table_names(src: &str) -> Result<Vec<String>> {
    let file = syn::parse_file(src).context("parsing the intent table")?;
    let mut names = Vec::new();
    for part in file.items.iter().filter_map(|item| match item {
        syn::Item::Const(c) if c.ident.to_string().ends_with("_ROWS") => Some(c),
        _ => None,
    }) {
        let syn::Expr::Reference(reference) = &*part.expr else {
            return Err(anyhow!("{} is not `&[…]`", part.ident));
        };
        let syn::Expr::Array(rows) = &*reference.expr else {
            return Err(anyhow!("{} is not `&[…]`", part.ident));
        };
        for row in &rows.elems {
            let syn::Expr::Tuple(t) = row else {
                return Err(anyhow!("a {} row is not a tuple", part.ident));
            };
            let Some(syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            })) = t.elems.first()
            else {
                return Err(anyhow!("a {} row does not start with a name", part.ident));
            };
            names.push(s.value());
        }
    }
    Ok(names)
}

/// The names of every part of the table, the files read in name order.
fn table_parts(root: &Path) -> Result<Vec<String>> {
    let dir = root.join(TABLE);
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("listing {TABLE}"))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("listing {TABLE}"))?;
    files.retain(|p| p.extension().is_some_and(|x| x == "rs"));
    files.sort();
    let mut names = Vec::new();
    for path in files {
        let src = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        names.extend(table_names(&src)?);
    }
    if names.is_empty() {
        return Err(anyhow!("no `const …_ROWS` table part in {TABLE}"));
    }
    Ok(names)
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
        &ipc_requests(&read(IPC_PROTOCOL)?, &read(IPC_COMMUNITY)?)?,
        &table_parts(root)?,
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
        let ipc =
            "pub enum IpcRequest { Unlock { passphrase: String }, Community(CommunityRequest), Lock }";
        let community = "pub enum CommunityRequest { Kick { target: String }, BanList }";
        assert_eq!(
            ipc_requests(ipc, community).unwrap(),
            names(&["Unlock", "Lock", "Kick", "BanList"])
        );
        let table = r#"pub(crate) const CORE_ROWS: &[(&str, Option<IntentKind>)] = &[("login", Some(IntentKind::Login)), ("Lock", None)]; pub const COMMAND_TABLE: &[&[(&str, Option<IntentKind>)]] = &[CORE_ROWS];"#;
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
