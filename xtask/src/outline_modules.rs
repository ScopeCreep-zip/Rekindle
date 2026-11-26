//! `cargo xtask outline-modules [--crate <name>]` (gate hardening item 1):
//! the deterministic fixer for `check-inline-modules`.
//!
//! Each file-level `mod name { … }` moves, body whole, to `<stem>_<name>.rs`
//! beside its parent (`<stem>` is the parent file's stem: `lib_tests.rs`,
//! `mod_tests.rs`, `foo_tests.rs`), and the block becomes
//! `#[path = "<stem>_<name>.rs"] mod name;` with the module's own attributes,
//! docs and visibility kept. A nested inline module is moved again from the
//! new file (`<stem>_<outer>_<inner>.rs`). The text is cut at the offsets
//! `rust_lex` finds and both files are re-parsed with `syn` (item counts must
//! match); the body is dedented only on lines that start outside a string
//! literal, then `cargo fmt -p` formats the touched members. Nothing else
//! changes.
//!
//! It refuses (and names the file) rather than guess: a target file that
//! already exists, a body holding a file-backed `mod x;` (its path would
//! resolve elsewhere), an inline module with a `#[path]`, an inline module
//! inside a function or other item, and a parent that is a Cargo
//! auto-discovered target root (`tests/`, `benches/`, `examples/`,
//! `src/bin/`), where a new sibling file would become its own target.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::inline_modules::{include_only, inline_modules_in, member_files};
use crate::rust_lex;

/// A parent's rewritten text and the files its modules moved to.
#[derive(Debug)]
pub(crate) struct Outlined {
    pub(crate) parent: String,
    /// (file name beside the parent, content).
    pub(crate) files: Vec<(String, String)>,
}

/// Dedent `body` by its common indentation, leaving every line that starts
/// inside a string literal untouched (`literals` are relative to `body`).
fn dedent(body: &str, literals: &[std::ops::Range<usize>]) -> String {
    let mut lines = Vec::new();
    let mut at = 0;
    for line in body.split_inclusive('\n') {
        let in_literal = literals.iter().any(|r| r.start < at && at < r.end);
        lines.push((line, in_literal));
        at += line.len();
    }
    let indent = lines
        .iter()
        .filter(|(l, lit)| !lit && !l.trim().is_empty())
        .map(|(l, _)| l.len() - l.trim_start_matches(' ').len())
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    for (line, lit) in lines {
        if lit {
            out.push_str(line);
        } else if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&line[indent..]);
        }
    }
    format!("{}\n", out.trim_start_matches('\n').trim_end())
}

/// Outline every inline module of `src` (a file whose stem is `stem`).
pub(crate) fn outline_source(src: &str, stem: &str) -> Result<Outlined> {
    let file = syn::parse_file(src).context("parsing")?;
    let top: Vec<&syn::ItemMod> = file
        .items
        .iter()
        .filter_map(|i| match i {
            syn::Item::Mod(m) if m.content.is_some() => Some(m),
            _ => None,
        })
        .collect();
    let all = inline_modules_in(src)?;
    let reachable = all.iter().filter(|f| !f.path.contains("::")).count();
    let outlined_top = top
        .iter()
        .filter(|m| !include_only(m.content.as_ref().map_or(&[][..], |c| &c.1[..])))
        .count();
    if reachable != outlined_top {
        bail!("an inline module sits inside a function or other item; move it by hand");
    }
    let lexed = rust_lex::lex(src);
    let blocks: Vec<rust_lex::ModBlock> = rust_lex::mod_blocks(src, &lexed)
        .into_iter()
        .filter(|b| b.depth == 0)
        .collect();
    let names: Vec<String> = top.iter().map(|m| m.ident.to_string()).collect();
    if blocks.iter().map(|b| &b.name).ne(names.iter()) {
        bail!("the module text and the parse disagree ({names:?})");
    }
    let mut parent = src.to_owned();
    let mut files = Vec::new();
    for (m, b) in top.iter().zip(&blocks).rev() {
        let items = m.content.as_ref().map_or(&[][..], |c| &c.1[..]);
        if include_only(items) {
            continue;
        }
        if m.attrs.iter().any(|a| a.path().is_ident("path")) {
            bail!(
                "inline `mod {}` carries a #[path]; move it by hand",
                m.ident
            );
        }
        if items
            .iter()
            .any(|i| matches!(i, syn::Item::Mod(c) if c.content.is_none()))
        {
            bail!(
                "inline `mod {}` declares a file-backed module; move it by hand",
                m.ident
            );
        }
        let name = b.name.as_str();
        let target = format!("{stem}_{name}.rs");
        let lits: Vec<_> = lexed
            .literals
            .iter()
            .filter(|r| r.start > b.open && r.end <= b.close)
            .map(|r| r.start - b.open - 1..r.end - b.open - 1)
            .collect();
        let body = dedent(&src[b.open + 1..b.close], &lits);
        let line_start = src[..b.mod_kw].rfind('\n').map_or(0, |n| n + 1);
        let indent_len = src[line_start..].len() - src[line_start..].trim_start().len();
        let indent = &src[line_start..line_start + indent_len];
        parent.replace_range(b.name_end..=b.close, ";");
        parent.insert_str(
            line_start + indent_len,
            &format!("#[path = \"{target}\"]\n{indent}"),
        );
        let reparsed = syn::parse_file(&body).with_context(|| format!("re-parsing {target}"))?;
        if reparsed.items.len() != items.len() {
            bail!("{target}: item count changed in the move");
        }
        let nested = outline_source(&body, &format!("{stem}_{name}"))?;
        files.push((target, nested.parent));
        files.extend(nested.files);
    }
    let reparsed = syn::parse_file(&parent).context("re-parsing the parent")?;
    if reparsed.items.len() != file.items.len() {
        bail!("the parent's item count changed in the move");
    }
    files.sort();
    Ok(Outlined { parent, files })
}

/// A parent whose new sibling Cargo would build as its own target.
fn in_target_root(path: &Path, member_dir: &Path) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    ["tests", "benches", "examples", "src/bin"]
        .iter()
        .any(|d| dir == member_dir.join(d))
}

/// Outline one parent file; `Ok(false)` when it has nothing to move.
fn outline_file(path: &Path, member_dir: &Path) -> Result<bool> {
    let src = std::fs::read_to_string(path)?;
    let Ok(found) = inline_modules_in(&src) else {
        return Ok(false);
    };
    if found.is_empty() {
        return Ok(false);
    }
    if in_target_root(path, member_dir) {
        bail!("a Cargo target root; a sibling file would become its own target");
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("no file stem"))?;
    let out = outline_source(&src, stem)?;
    let dir = path.parent().ok_or_else(|| anyhow!("no parent dir"))?;
    for (name, _) in &out.files {
        if dir.join(name).exists() {
            bail!("{name} already exists; refusing to overwrite it");
        }
    }
    for (name, content) in &out.files {
        std::fs::write(dir.join(name), content)?;
    }
    std::fs::write(path, &out.parent)?;
    Ok(true)
}

pub(crate) fn outline_modules(root: &Path, krate: Option<&str>) -> Result<()> {
    let members = crate::inline_modules::member_dirs(root)?;
    let files = member_files(root)?;
    if let Some(k) = krate {
        if !files.contains_key(k) {
            bail!("no workspace member named {k}");
        }
    }
    let mut touched = BTreeSet::new();
    let mut refused = 0usize;
    for (member, paths) in &files {
        if krate.is_some_and(|k| k != member) {
            continue;
        }
        let dir = members
            .iter()
            .find(|(n, _)| n == member)
            .map(|(_, d)| d.clone())
            .ok_or_else(|| anyhow!("member {member} has no directory"))?;
        for path in paths {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string();
            match outline_file(path, &dir) {
                Ok(true) => {
                    println!("    outlined {rel}");
                    touched.insert(member.clone());
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("    ✗ {rel}: {e:#}");
                    refused += 1;
                }
            }
        }
    }
    for member in &touched {
        let status =
            std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
                .args(["fmt", "-p", member])
                .current_dir(root)
                .status()
                .context("running cargo fmt")?;
        if !status.success() {
            bail!("cargo fmt -p {member} failed");
        }
    }
    if refused > 0 {
        bail!("{refused} file(s) refused; see the lines above");
    }
    Ok(())
}

#[cfg(test)]
#[path = "outline_modules_tests.rs"]
mod tests;
