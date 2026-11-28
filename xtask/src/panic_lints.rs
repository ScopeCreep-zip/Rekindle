//! `cargo xtask check-panic-lints` (gate hardening item 3(a), owner
//! 2026-11-26: "we need xtask to enforce strong appsec").
//!
//! A crate that decodes peer input must not panic on it: a panic there is a
//! remote crash. Each crate in [`PEER_INPUT_CRATES`] keeps the six clippy
//! lints of [`PANIC_LINTS`] at `deny` for its non-test code
//! (`cfg_attr(not(test), deny(…))`; test code is exempt by `cfg`, never by an
//! `allow`), in one of two shapes at its lib root:
//!
//! - a crate attribute `#![cfg_attr(not(test), deny(…))]`; or
//! - where the root also holds generated Cap'n Proto modules (which a crate
//!   attribute would reach, and which are not ours to change), every root
//!   module carries the attribute itself, written directly or through a
//!   `macro_rules!` whose expansion carries it; the only other root macro
//!   calls allowed are the generated-module macro (`include!` of `OUT_DIR`).
//!
//! Anywhere in the crate, an `allow`, `expect` or `warn` naming one of the
//! lints is a failure. [`PEER_INPUT_CRATES`] is the scope of the conversion
//! (one crate per commit, peer-input order), not an exemption list: it only
//! grows.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use quote::ToTokens;
use syn::visit::Visit;

/// The panic-class lints a peer-input crate denies.
pub(crate) const PANIC_LINTS: [&str; 6] = [
    "indexing_slicing",
    "arithmetic_side_effects",
    "unwrap_used",
    "expect_used",
    "panic",
    "string_slice",
];

/// The crates converted so far.
pub(crate) const PEER_INPUT_CRATES: [&str; 2] = ["rekindle-codec", "rekindle-protocol"];

fn squash(tokens: &str) -> String {
    tokens.chars().filter(|c| !c.is_whitespace()).collect()
}

/// True when `tokens` hold `cfg_attr(not(test), deny(…))` naming every lint.
pub(crate) fn carries_lints(tokens: &str) -> bool {
    let t = squash(tokens);
    let Some(at) = t.find("cfg_attr(not(test),deny(") else {
        return false;
    };
    let rest = &t[at..];
    PANIC_LINTS.iter().all(|lint| {
        let name = format!("clippy::{lint}");
        rest.split([',', ')', '(']).any(|part| part == name)
    })
}

fn macro_name(m: &syn::Macro) -> String {
    m.path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

/// What is missing at a lib root's source (empty when the lints hold).
pub(crate) fn root_findings(src: &str) -> Result<Vec<String>> {
    let file = syn::parse_file(src)?;
    if file
        .attrs
        .iter()
        .any(|a| carries_lints(&a.to_token_stream().to_string()))
    {
        return Ok(Vec::new());
    }
    let mut lint_macros = Vec::new();
    let mut generated_macros = Vec::new();
    for item in &file.items {
        if let syn::Item::Macro(m) = item {
            if let Some(ident) = &m.ident {
                let body = m.mac.tokens.to_string();
                if carries_lints(&body) && squash(&body).contains("pubmod$") {
                    lint_macros.push(ident.to_string());
                } else if body.contains("include") && body.contains("OUT_DIR") {
                    generated_macros.push(ident.to_string());
                }
            }
        }
    }
    let mut findings = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Mod(m)
                if !m
                    .attrs
                    .iter()
                    .any(|a| carries_lints(&a.to_token_stream().to_string())) =>
            {
                findings.push(format!("`mod {}` without the panic lints", m.ident));
            }
            syn::Item::Macro(m) if m.ident.is_none() => {
                let name = macro_name(&m.mac);
                if !lint_macros.contains(&name) && !generated_macros.contains(&name) {
                    findings.push(format!(
                        "`{name}!` at the root is neither a panic-lint module macro nor a \
                         generated-module macro"
                    ));
                }
            }
            _ => {}
        }
    }
    let declares = file.items.iter().any(|i| match i {
        syn::Item::Mod(_) => true,
        syn::Item::Macro(m) => m.ident.is_none() && lint_macros.contains(&macro_name(&m.mac)),
        _ => false,
    });
    if !declares {
        findings.push(format!(
            "no `#![cfg_attr(not(test), deny(clippy::{}))]` at the crate root",
            PANIC_LINTS.join(", clippy::")
        ));
    }
    Ok(findings)
}

#[derive(Default)]
struct Lowered(Vec<String>);

impl Lowered {
    fn scan(&mut self, tokens: &str) {
        let t = squash(tokens);
        let lowers = ["allow(", "expect(", "warn("].iter().any(|l| t.contains(l));
        if lowers {
            for lint in PANIC_LINTS {
                let name = format!("clippy::{lint}");
                if t.split([',', ')', '(']).any(|part| part == name) {
                    self.0.push(format!("`{name}` lowered in `{t}`"));
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for Lowered {
    fn visit_attribute(&mut self, a: &'ast syn::Attribute) {
        self.scan(&a.to_token_stream().to_string());
        syn::visit::visit_attribute(self, a);
    }

    // A macro body is unparsed tokens: an attribute written inside one
    // (a `macro_rules!` that declares modules) is read as text.
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        self.scan(&m.tokens.to_string());
        syn::visit::visit_macro(self, m);
    }
}

/// Every attribute in `src` that lowers one of the lints.
pub(crate) fn lowered_in(src: &str) -> Result<Vec<String>> {
    let file = syn::parse_file(src)?;
    let mut v = Lowered::default();
    v.visit_file(&file);
    Ok(v.0)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

pub(crate) fn check_panic_lints(root: &Path) -> Result<()> {
    let mut total = 0usize;
    for krate in PEER_INPUT_CRATES {
        let dir = root.join("crates").join(krate);
        let lib = dir.join("src").join("lib.rs");
        let src = fs::read_to_string(&lib).with_context(|| format!("reading {}", lib.display()))?;
        for f in root_findings(&src)? {
            eprintln!("    ✗ crates/{krate}/src/lib.rs: {f}");
            total += 1;
        }
        let mut files = Vec::new();
        rust_files(&dir, &mut files)?;
        files.sort();
        for file in files {
            let src = fs::read_to_string(&file)?;
            for f in lowered_in(&src)? {
                let rel = file.strip_prefix(root).unwrap_or(&file).display();
                eprintln!("    ✗ {rel}: {f}");
                total += 1;
            }
        }
    }
    if total > 0 {
        return Err(anyhow!(
            "{total} panic-lint finding(s): peer-input crates deny clippy::{} in non-test code",
            PANIC_LINTS.join(", clippy::")
        ));
    }
    println!("    panic lints on: {}", PEER_INPUT_CRATES.join(", "));
    Ok(())
}

#[cfg(test)]
#[path = "panic_lints_tests.rs"]
mod tests;
