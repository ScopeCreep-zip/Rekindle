//! `check-no-shape-ipc`: the webview never shapes a window (plan SK0 S4).
//!
//! A window's silhouette and its click-through state are computed in Rust
//! from the validated skin package and the window size
//! (`src-tauri/src/windows/shape/`). If an IPC command took a region, a
//! rectangle list or a cursor-ignore flag, an XSS in any window could make
//! a window invisible or pass its clicks to whatever lies behind it (UI
//! redressing; threat model W4). This gate walks every `#[tauri::command]`
//! function under `src-tauri/src` and fails on one whose name, parameter
//! names or parameter types speak of regions, shapes, silhouettes,
//! rectangles, or ignoring the cursor or mouse.

use std::path::Path;

use anyhow::Result;
use quote::ToTokens;
use syn::visit::Visit;

use crate::syn_scan::{scan_rust_dirs, verdict, visit};

/// Words that, alone, name window geometry. "region" alone is not one: the
/// skin editor's `import_skin_region` imports a stylable UI region of a
/// skin (`src/styles/regions.toml`), not a window region, so a region is
/// forbidden only next to a word that makes it a window's ([`PAIRS`]).
const WORDS: &[&str] = &[
    "rgn",
    "hrgn",
    "shape",
    "shapes",
    "shaped",
    "silhouette",
    "rect",
    "rects",
    "clickthrough",
];

/// Adjacent word pairs, inside one identifier, that name a window region or
/// click-through (adjacent, so `subscription_events::Cursor` is not
/// "cursor events").
const PAIRS: &[(&str, &str)] = &[
    ("window", "region"),
    ("input", "region"),
    ("visible", "region"),
    ("bounding", "region"),
    ("hit", "region"),
    ("ignore", "cursor"),
    ("ignore", "mouse"),
    ("ignores", "mouse"),
    ("cursor", "events"),
    ("mouse", "events"),
    ("click", "through"),
    ("pass", "through"),
    ("hit", "test"),
];

/// The lower-case words of an identifier or a type's tokens: split on
/// anything not alphanumeric and at camel-case humps.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in text.chars() {
        if !c.is_ascii_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        word.push(c.to_ascii_lowercase());
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// The geometry term `text` (an identifier or a type's tokens) speaks
/// of, if any.
fn forbidden(text: &str) -> Option<String> {
    let identifiers = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|ident| !ident.is_empty());
    for ident in identifiers {
        let words = words(ident);
        if let Some(w) = words.iter().find(|w| WORDS.contains(&w.as_str())) {
            return Some(w.clone());
        }
        if let Some((a, b)) = words
            .windows(2)
            .find(|pair| PAIRS.contains(&(pair[0].as_str(), pair[1].as_str())))
            .map(|pair| (pair[0].clone(), pair[1].clone()))
        {
            return Some(format!("{a} {b}"));
        }
    }
    None
}

fn is_command(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let path = a.path();
        path.segments.last().is_some_and(|s| s.ident == "command")
            && (path.segments.len() == 1
                || path.segments.first().is_some_and(|s| s.ident == "tauri"))
    })
}

struct Finder<'a> {
    rel: &'a str,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn check(&mut self, name: &str, what: &str, text: &str) {
        if let Some(term) = forbidden(text) {
            self.hits.push(format!(
                "{}  command `{name}`: {what} `{text}` names \"{term}\"",
                self.rel
            ));
        }
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if is_command(&item.attrs) {
            let name = item.sig.ident.to_string();
            self.check(&name, "name", &name);
            for input in &item.sig.inputs {
                let syn::FnArg::Typed(arg) = input else {
                    continue;
                };
                let pat = arg.pat.to_token_stream().to_string();
                self.check(&name, "parameter", &pat);
                let ty = arg.ty.to_token_stream().to_string();
                self.check(&name, "parameter type", &ty);
            }
        }
        syn::visit::visit_item_fn(self, item);
    }
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

pub(crate) fn check_no_shape_ipc(root: &Path) -> Result<()> {
    let hits = scan_rust_dirs(root, &["src-tauri/src"], scan)?;
    verdict(
        &hits,
        "IPC command(s) take window-shape or click-through input. The \
         silhouette is computed in Rust (src-tauri/src/windows/shape); no \
         command may take a region or a cursor-ignore flag (plan SK0 S4).",
    )
}

#[cfg(test)]
#[path = "no_shape_ipc_tests.rs"]
mod tests;
