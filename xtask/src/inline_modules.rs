//! `cargo xtask check-inline-modules` (gate hardening item 1, owner
//! 2026-11-26: "building with modules and never inline items").
//!
//! Every workspace `.rs` file is parsed with `syn`; any `mod name { … }`
//! with a body is a failure, test modules included (tests live in
//! `<stem>_tests.rs`, declared `#[cfg(test)] #[path = "…"] mod tests;`, the
//! rustc tidy rule). One structural property is not an inline module: a body
//! made only of `include!(…)` items keeps its code in another file (the Cap'n
//! Proto generated modules, CLAUDE.md). No baseline, no list.
//!
//! Scope: every workspace member's directory, the vendored
//! `tauri-plugin-wdio-webdriver` included: it is a member we patch and build,
//! it inherits `[workspace.lints]`, and the design scopes all of `crates/**`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::cargo_meta::Metadata;
use crate::rust_lex;
use crate::walk_source_files;

/// One inline module.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    /// 1-based line of its `mod` keyword (0 when the text could not be
    /// located).
    pub(crate) line: usize,
    /// `outer::inner` path from the file's root.
    pub(crate) path: String,
}

/// True when `items` is non-empty and only `include!(…)` invocations.
pub(crate) fn include_only(items: &[syn::Item]) -> bool {
    !items.is_empty()
        && items.iter().all(|i| {
            matches!(i, syn::Item::Macro(m)
                if m.mac.path.segments.last().is_some_and(|s| s.ident == "include"))
        })
}

#[derive(Default)]
struct Collect {
    stack: Vec<String>,
    found: Vec<(String, String)>,
}

impl<'ast> Visit<'ast> for Collect {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        self.stack.push(m.ident.to_string());
        if let Some((_, items)) = &m.content {
            if !include_only(items) {
                self.found
                    .push((m.ident.to_string(), self.stack.join("::")));
            }
        }
        syn::visit::visit_item_mod(self, m);
        self.stack.pop();
    }
}

/// The inline modules of one file's source, in source order.
pub(crate) fn inline_modules_in(src: &str) -> Result<Vec<Finding>> {
    let file = syn::parse_file(src)?;
    let mut c = Collect::default();
    c.visit_file(&file);
    let lexed = rust_lex::lex(src);
    let blocks = rust_lex::mod_blocks(src, &lexed);
    let mut next = 0;
    Ok(c.found
        .into_iter()
        .map(|(name, path)| {
            let line = match blocks[next..].iter().position(|b| b.name == name) {
                Some(p) => {
                    next += p + 1;
                    rust_lex::line_of(src, blocks[next - 1].mod_kw)
                }
                None => 0,
            };
            Finding { line, path }
        })
        .collect())
}

/// Every workspace member as (package name, directory), deepest first so a
/// file is owned by the innermost member that contains it.
pub(crate) fn member_dirs(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let meta = Metadata::load(root)?;
    let mut out: Vec<(String, PathBuf)> = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .filter_map(|p| Some((p.name.clone(), p.manifest_path.parent()?.to_path_buf())))
        .collect();
    out.sort_by_key(|(_, d)| std::cmp::Reverse(d.components().count()));
    Ok(out)
}

/// Every `.rs` file per member (a file belongs to its innermost member).
pub(crate) fn member_files(root: &Path) -> Result<BTreeMap<String, Vec<PathBuf>>> {
    let members = member_dirs(root)?;
    let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (name, dir) in &members {
        let mut files: Vec<PathBuf> = walk_source_files(dir, &["rs"])?
            .into_iter()
            .filter(|f| {
                members
                    .iter()
                    .find(|(_, d)| f.starts_with(d))
                    .is_some_and(|(n, _)| n == name)
            })
            .collect();
        files.sort();
        out.insert(name.clone(), files);
    }
    Ok(out)
}

/// Per-member findings as (relative path, finding).
pub(crate) fn scan(root: &Path) -> Result<BTreeMap<String, Vec<(String, Finding)>>> {
    let mut out = BTreeMap::new();
    for (name, files) in member_files(root)? {
        let mut found = Vec::new();
        for path in files {
            let src = std::fs::read_to_string(&path)?;
            // A file that does not parse is cargo's to report, not this gate's.
            let Ok(list) = inline_modules_in(&src) else {
                continue;
            };
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            found.extend(list.into_iter().map(|f| (rel.clone(), f)));
        }
        out.insert(name, found);
    }
    Ok(out)
}

pub(crate) fn check_inline_modules(root: &Path) -> Result<()> {
    let by_member = scan(root)?;
    let mut total = 0usize;
    for (member, found) in &by_member {
        for (rel, f) in found {
            eprintln!("    ✗ {rel}:{} inline `mod {} {{ … }}`", f.line, f.path);
        }
        if !found.is_empty() {
            println!("    {member}: {} inline module(s)", found.len());
        }
        total += found.len();
    }
    if total > 0 {
        return Err(anyhow!(
            "{total} inline module(s): move each body to `<stem>_<name>.rs` with \
             `#[path = \"…\"] mod name;` (`cargo xtask outline-modules`)"
        ));
    }
    println!("    no inline modules");
    Ok(())
}

#[cfg(test)]
#[path = "inline_modules_tests.rs"]
mod tests;
