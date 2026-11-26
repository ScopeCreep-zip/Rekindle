//! `check-test-fixtures`: a secret-shaped literal in test code comes from the canonical fixture
//! crate `rekindle-test-fixtures`, by path, never as a copy (owner ruling 2026-11-26).
//!
//! A literal is secret-shaped when a credential scanner reads it as a credential. The rule
//! mirrors gitleaks `generic-api-key`, the rule that flagged the history values: a string of
//! 10–150 characters from `[A-Za-z0-9_.=+/-]` with a digit and Shannon entropy of at least 3.5,
//! bound to a credential name (`access`, `auth`, `api`, `credential`, `creds`, `key`, `passwd`,
//! `password`, `passphrase`, `secret`, `token`, `seed`) by `let`, `const`, `static`, a struct
//! field, an assignment, or a `name: value` / `name = value` pair inside a macro such as
//! `json!`. A literal with a provider token prefix (AWS, GitHub, GitLab, Slack, Stripe) or a
//! PEM private-key header is secret-shaped wherever it stands.
//!
//! Rust test code is the `tests/` trees, `tests.rs`, `*_tests.rs` and `test_fixtures.rs` files,
//! and in every other file the items under `#[cfg(test)]` or a `#[test]`-style attribute.
//! TypeScript test code is `e2e/**` and `src/**/*.test.ts(x)`. The frontend keeps no static
//! secret-shaped fixture: its suites generate each passphrase at run time (`e2e/app/user.ts`),
//! so any secret-shaped literal there fails.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use quote::ToTokens;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, Lit, Member, Pat};

use crate::walk_source_files;

/// The canonical fixture crate, the one place these values are written.
const CANONICAL: &str = "crates/rekindle-test-fixtures/";

const CREDENTIAL_WORDS: &[&str] = &[
    "access",
    "auth",
    "api",
    "credential",
    "creds",
    "key",
    "passwd",
    "password",
    "passphrase",
    "secret",
    "token",
    "seed",
];

/// `(prefix, minimum alphanumeric run after it)` of provider tokens.
const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("AKIA", 16),
    ("ghp_", 36),
    ("gho_", 36),
    ("ghs_", 36),
    ("ghu_", 36),
    ("ghr_", 36),
    ("github_pat_", 22),
    ("glpat-", 20),
    ("xoxb-", 10),
    ("xoxp-", 10),
    ("sk_live_", 16),
];

pub(crate) fn check_test_fixtures(root: &Path) -> Result<()> {
    let mut findings = Vec::new();
    for dir in ["crates", "src-tauri", "xtask", "contract"] {
        for path in walk_source_files(&root.join(dir), &["rs"])? {
            let rel = rel_path(root, &path);
            if rel.starts_with(CANONICAL) || rel.contains("/vendor/") {
                continue;
            }
            let src = std::fs::read_to_string(&path)?;
            let found = scan_rust(&rel, &src).map_err(|e| anyhow!("{rel}: {e}"))?;
            findings.extend(found.into_iter().map(|f| format!("{rel}: {f}")));
        }
    }
    let ts = walk_source_files(&root.join("e2e"), &["ts"])?
        .into_iter()
        .chain(
            walk_source_files(&root.join("src"), &["ts", "tsx"])?
                .into_iter()
                .filter(|p| is_ts_test_file(&rel_path(root, p))),
        );
    for path in ts {
        let rel = rel_path(root, &path);
        let src = std::fs::read_to_string(&path)?;
        findings.extend(scan_ts(&src).into_iter().map(|f| format!("{rel}: {f}")));
    }
    if findings.is_empty() {
        return Ok(());
    }
    for f in &findings {
        eprintln!("    {f}");
    }
    bail!(
        "{} secret-shaped literal(s) in test code; reference the canonical value in \
         rekindle-test-fixtures (or add it there, carrying the ramp marker) instead",
        findings.len()
    )
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn is_rust_test_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    rel.split('/').any(|c| c == "tests")
        || name == "tests.rs"
        || name == "test_fixtures.rs"
        || name.ends_with("_tests.rs")
}

fn is_ts_test_file(rel: &str) -> bool {
    rel.ends_with(".test.ts") || rel.ends_with(".test.tsx") || rel.contains("/__tests__/")
}

/// Shannon entropy in bits per character.
fn entropy(value: &str) -> f64 {
    let mut counts = [0usize; 256];
    for b in value.bytes() {
        counts[usize::from(b)] += 1;
    }
    let n = f64::from(u32::try_from(value.len()).unwrap_or(u32::MAX));
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(u32::try_from(c).unwrap_or(u32::MAX)) / n;
            -p * p.log2()
        })
        .sum()
}

fn credential_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    CREDENTIAL_WORDS.iter().any(|w| name.contains(w))
}

fn high_entropy(value: &str) -> bool {
    (10..=150).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.=+/-".contains(&b))
        && value.bytes().any(|b| b.is_ascii_digit())
        && entropy(value) >= 3.5
}

fn token_shaped(value: &str) -> bool {
    (value.contains("-----BEGIN") && value.contains("PRIVATE KEY-----"))
        || TOKEN_PREFIXES.iter().any(|(prefix, min)| {
            value.match_indices(prefix).any(|(i, _)| {
                value[i + prefix.len()..]
                    .bytes()
                    .take_while(u8::is_ascii_alphanumeric)
                    .count()
                    >= *min
            })
        })
}

/// The finding for `value` bound to `name`, if it is secret-shaped.
fn judge(name: &str, value: &str) -> Option<String> {
    let shown: String = value.chars().take(6).collect();
    if token_shaped(value) {
        return Some(format!("\"{shown}…\" is token-shaped"));
    }
    (credential_name(name) && high_entropy(value))
        .then(|| format!("{name} = \"{shown}…\" is secret-shaped"))
}

/// `(binding name, value)` for every one-line string literal in `text`; the name is the
/// identifier or quoted key before a `:` or `=` in front of the literal, or empty.
fn text_literals(text: &str) -> Vec<(String, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let q = b[i];
        if !matches!(q, b'"' | b'\'' | b'`') {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && b[j] != q && b[j] != b'\n' {
            j += if b[j] == b'\\' { 2 } else { 1 };
        }
        match text.get(i + 1..j) {
            Some(value) if j < b.len() && b[j] == q => {
                out.push((binding_name(&text[..i]), value.to_string()));
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    out
}

fn binding_name(before: &str) -> String {
    let t = before.trim_end();
    let Some(lhs) = t.strip_suffix(':').or_else(|| t.strip_suffix('=')) else {
        return String::new();
    };
    if lhs.ends_with(['=', '!', '<', '>', ':']) {
        return String::new();
    }
    let lhs = lhs.trim_end();
    if let Some(quoted) = lhs.strip_suffix(['"', '\'']) {
        return quoted.rsplit(['"', '\'']).next().unwrap_or("").to_string();
    }
    lhs.rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .next()
        .unwrap_or("")
        .to_string()
}

fn scan_ts(src: &str) -> Vec<String> {
    text_literals(src)
        .into_iter()
        .filter_map(|(name, value)| judge(&name, &value))
        .collect()
}

fn scan_rust(rel: &str, src: &str) -> Result<Vec<String>> {
    let file = syn::parse_file(src)?;
    let mut scan = Scan {
        in_test: is_rust_test_file(rel),
        found: Vec::new(),
    };
    scan.visit_file(&file);
    Ok(scan.found)
}

struct Scan {
    in_test: bool,
    found: Vec<String>,
}

fn is_test_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        if a.path().is_ident("cfg") {
            let meta = a.meta.to_token_stream().to_string();
            return !meta.contains("not")
                && meta
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|w| w == "test");
        }
        a.path().segments.last().is_some_and(|s| s.ident == "test")
    })
}

/// The string literal an initialiser reduces to (`"…"`, `"…".into()`, `String::from("…")`).
fn literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(l) => match &l.lit {
            Lit::Str(s) => Some(s.value()),
            _ => None,
        },
        Expr::Reference(r) => literal(&r.expr),
        Expr::Paren(p) => literal(&p.expr),
        Expr::Group(g) => literal(&g.expr),
        Expr::MethodCall(m) => literal(&m.receiver),
        Expr::Call(c) if c.args.len() == 1 => literal(&c.args[0]),
        _ => None,
    }
}

fn pat_name(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Ident(p) => Some(p.ident.to_string()),
        Pat::Type(t) => pat_name(&t.pat),
        _ => None,
    }
}

impl Scan {
    fn binding(&mut self, name: &str, expr: &Expr) {
        // A token-shaped literal is reported once, by `visit_lit_str`.
        match literal(expr) {
            Some(value) if self.in_test && !token_shaped(&value) => {
                self.found.extend(judge(name, &value));
            }
            _ => {}
        }
    }

    fn scoped(&mut self, attrs: &[Attribute], walk: impl FnOnce(&mut Self)) {
        let was = self.in_test;
        self.in_test |= is_test_attr(attrs);
        walk(self);
        self.in_test = was;
    }
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        self.scoped(&m.attrs, |s| visit::visit_item_mod(s, m));
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.scoped(&f.attrs, |s| visit::visit_item_fn(s, f));
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        if let (Some(name), Some(init)) = (pat_name(&l.pat), &l.init) {
            self.binding(&name, &init.expr);
        }
        visit::visit_local(self, l);
    }

    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        self.binding(&c.ident.to_string(), &c.expr);
        visit::visit_item_const(self, c);
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        self.binding(&s.ident.to_string(), &s.expr);
        visit::visit_item_static(self, s);
    }

    fn visit_field_value(&mut self, f: &'ast syn::FieldValue) {
        if let Member::Named(name) = &f.member {
            self.binding(&name.to_string(), &f.expr);
        }
        visit::visit_field_value(self, f);
    }

    fn visit_expr_assign(&mut self, a: &'ast syn::ExprAssign) {
        let name = match &*a.left {
            Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
            Expr::Field(f) => match &f.member {
                Member::Named(n) => Some(n.to_string()),
                Member::Unnamed(_) => None,
            },
            _ => None,
        };
        if let Some(name) = name {
            self.binding(&name, &a.right);
        }
        visit::visit_expr_assign(self, a);
    }

    fn visit_lit_str(&mut self, s: &'ast syn::LitStr) {
        if self.in_test && token_shaped(&s.value()) {
            self.found.extend(judge("", &s.value()));
        }
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if self.in_test {
            for (name, value) in text_literals(&m.tokens.to_string()) {
                self.found.extend(judge(&name, &value));
            }
        }
        visit::visit_macro(self, m);
    }
}

#[cfg(test)]
#[path = "test_fixtures_tests.rs"]
mod tests;
