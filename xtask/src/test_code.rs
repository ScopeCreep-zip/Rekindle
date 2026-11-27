//! Which Rust files are test code, known from how each file is declared
//! (gate hardening item 1, lead ruling 2026-11-26).
//!
//! A file is test code iff it sits under a workspace member's `tests/` or
//! `benches/` directory, is the root of a test or bench target (`[[test]]`,
//! `[[bench]]`), or is loaded by a `mod` item that carries `#[cfg(test)]`
//! (or a test attribute) or that sits inside test code. The module tree is
//! walked from every target root of every member, resolving `mod name;` the
//! way rustc does: `name.rs` or `name/mod.rs` in the directory a file owns
//! (its own for a crate root, a `mod.rs` or a `#[path]`-loaded file, its
//! `<stem>/` otherwise, plus any inline modules around the item), and a
//! `#[path = "…"]` relative to the declaring file's directory (or, inside an
//! inline module, to that module's directory). A file declared from both
//! test and shipped code is shipped code. The file name never decides: a
//! `foo_tests.rs` declared without `#[cfg(test)]` is shipped code.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;

use crate::walk_source_files;

/// Is `attr` `#[test]`, `#[<runtime>::test]` or `#[cfg(test)]`?
pub(crate) fn is_test_attr(attr: &syn::Attribute) -> bool {
    let path = attr.path();
    if path.is_ident("test") || path.segments.last().is_some_and(|s| s.ident == "test") {
        return true;
    }
    path.is_ident("cfg")
        && attr
            .parse_args::<syn::Meta>()
            .is_ok_and(|meta| meta.path().is_ident("test"))
}

/// A target's root file and whether the target is a test or bench.
pub(crate) struct Root {
    pub(crate) path: PathBuf,
    pub(crate) test: bool,
}

/// A file reached from a target root.
struct Reached {
    test: bool,
    /// Whether the file owns its directory (crate root, `mod.rs`, `#[path]`).
    owns_dir: bool,
    /// The file whose `mod` item loaded it (`None` for a target root).
    declarer: Option<PathBuf>,
}

/// The workspace's test code.
#[derive(Default)]
pub(crate) struct TestCode {
    files: BTreeMap<PathBuf, Reached>,
    /// Members' `tests/` and `benches/` directories.
    dirs: Vec<PathBuf>,
}

impl TestCode {
    /// The workspace's test code, walked once per run.
    pub(crate) fn load(root: &Path) -> Result<&'static TestCode> {
        static CODE: OnceLock<TestCode> = OnceLock::new();
        if let Some(code) = CODE.get() {
            return Ok(code);
        }
        let code = Self::scan_workspace(root)?;
        Ok(CODE.get_or_init(|| code))
    }

    fn scan_workspace(root: &Path) -> Result<Self> {
        let meta = crate::cargo_meta::Metadata::load(root)?;
        let mut sources = BTreeMap::new();
        let mut roots = Vec::new();
        let mut dirs = Vec::new();
        for package in meta.members() {
            let dir = canonical(package.manifest_path.parent().unwrap_or(Path::new("")));
            let mut walked = vec![dir.join("src"), dir.join("examples")];
            dirs.extend([dir.join("tests"), dir.join("benches")]);
            walked.extend(dirs[dirs.len() - 2..].iter().cloned());
            for target in &package.targets {
                let path = canonical(&target.src_path);
                walked.extend(path.parent().map(Path::to_path_buf));
                let test = target.kind.iter().any(|k| k == "test" || k == "bench");
                roots.push(Root { path, test });
            }
            walked.sort();
            walked.dedup();
            for walk_dir in walked {
                for path in walk_source_files(&walk_dir, &["rs"])? {
                    if let std::collections::btree_map::Entry::Vacant(slot) =
                        sources.entry(canonical(&path))
                    {
                        let src = std::fs::read_to_string(slot.key())?;
                        slot.insert(src);
                    }
                }
            }
        }
        Ok(Self::from_sources(&sources, &roots, dirs))
    }

    /// Walk the module trees of `roots` over `sources` (path → text).
    pub(crate) fn from_sources(
        sources: &BTreeMap<PathBuf, String>,
        roots: &[Root],
        dirs: Vec<PathBuf>,
    ) -> Self {
        let mut code = Self {
            files: BTreeMap::new(),
            dirs,
        };
        let mut queue = VecDeque::new();
        for r in roots {
            code.reach(&mut queue, r.path.clone(), r.test, true, None);
        }
        while let Some(path) = queue.pop_front() {
            let (Some(src), Some(reached)) = (sources.get(&path), code.files.get(&path)) else {
                continue;
            };
            let Ok(parsed) = syn::parse_file(src) else {
                continue;
            };
            let file_dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
            let child_dir = if reached.owns_dir {
                file_dir.clone()
            } else {
                file_dir.join(path.file_stem().unwrap_or_default())
            };
            let scope = Scope {
                file_dir: &file_dir,
                sources,
            };
            let mut found = Vec::new();
            scope.declared(&parsed.items, &child_dir, reached.test, true, &mut found);
            for (child, test, owns_dir) in found {
                if sources.contains_key(&child) {
                    code.reach(&mut queue, child, test, owns_dir, Some(path.clone()));
                }
            }
        }
        code
    }

    /// Record a declaration of `path`; a shipped declaration outranks a test one.
    fn reach(
        &mut self,
        queue: &mut VecDeque<PathBuf>,
        path: PathBuf,
        test: bool,
        owns_dir: bool,
        declarer: Option<PathBuf>,
    ) {
        match self.files.get_mut(&path) {
            None => {
                self.files.insert(
                    path.clone(),
                    Reached {
                        test,
                        owns_dir,
                        declarer,
                    },
                );
                queue.push_back(path);
            }
            Some(r) if r.test && !test => {
                r.test = false;
                r.declarer = declarer;
                queue.push_back(path);
            }
            Some(_) => {}
        }
    }

    /// Is `path` test code?
    pub(crate) fn is_test(&self, path: &Path) -> bool {
        let path = canonical(path);
        self.dirs.iter().any(|d| path.starts_with(d))
            || self.files.get(&path).is_some_and(|r| r.test)
    }

    /// The file whose `mod` item loaded `path`.
    pub(crate) fn declarer(&self, path: &Path) -> Option<&Path> {
        self.files.get(&canonical(path))?.declarer.as_deref()
    }
}

/// The declaring file a module walk is in.
struct Scope<'s> {
    file_dir: &'s Path,
    sources: &'s BTreeMap<PathBuf, String>,
}

impl Scope<'_> {
    /// Push `(file, test, owns_dir)` for every non-inline `mod` in `items`,
    /// `dir` being where their files live and `top` whether `items` are the
    /// file's own (not inside an inline module).
    fn declared(
        &self,
        items: &[syn::Item],
        dir: &Path,
        test: bool,
        top: bool,
        out: &mut Vec<(PathBuf, bool, bool)>,
    ) {
        // A `macro_rules!` that declares `mod $name;` (a peer-input crate's
        // panic-lint module macro, `cargo xtask check-panic-lints`): each call
        // `m!(name)` declares a shipped module `name`.
        let module_macros: Vec<String> = items
            .iter()
            .filter_map(|i| match i {
                syn::Item::Macro(m) => m.ident.as_ref().filter(|_| {
                    let body: String = m.mac.tokens.to_string().split_whitespace().collect();
                    body.contains("mod$")
                }),
                _ => None,
            })
            .map(ToString::to_string)
            .collect();
        for item in items {
            if let syn::Item::Macro(call) = item {
                let declares = call.ident.is_none()
                    && call
                        .mac
                        .path
                        .segments
                        .last()
                        .is_some_and(|s| module_macros.contains(&s.ident.to_string()));
                if let Some(name) = declares
                    .then(|| call.mac.parse_body::<syn::Ident>().ok())
                    .flatten()
                {
                    self.file_module(dir, &name.to_string(), test, out);
                }
                continue;
            }
            let syn::Item::Mod(m) = item else {
                continue;
            };
            let name = m.ident.to_string();
            let test = test || m.attrs.iter().any(is_test_attr);
            if let Some((_, inner)) = &m.content {
                self.declared(inner, &dir.join(&name), test, false, out);
            } else if let Some(p) = path_attr(&m.attrs) {
                let base = if top { self.file_dir } else { dir };
                out.push((normalise(&base.join(p)), test, true));
            } else {
                self.file_module(dir, &name, test, out);
            }
        }
    }

    /// Push the file of module `name` declared without `#[path]` in `dir`.
    fn file_module(
        &self,
        dir: &Path,
        name: &str,
        test: bool,
        out: &mut Vec<(PathBuf, bool, bool)>,
    ) {
        let file = dir.join(format!("{name}.rs"));
        if self.sources.contains_key(&file) {
            out.push((file, test, false));
        } else {
            out.push((dir.join(name).join("mod.rs"), test, true));
        }
    }
}

/// The value of a `#[path = "…"]` attribute.
pub(crate) fn path_attr(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|a| {
        let syn::Meta::NameValue(nv) = &a.meta else {
            return None;
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) = &nv.value
        else {
            return None;
        };
        nv.path.is_ident("path").then(|| s.value())
    })
}

/// `path` with `.` and `..` resolved lexically.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "test_code_tests.rs"]
mod tests;
