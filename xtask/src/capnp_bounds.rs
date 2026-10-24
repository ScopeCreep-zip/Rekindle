//! `check-capnp-bounds`: every Cap'n Proto message is read with a traversal
//! bound (plan F4, research batch 8 AO §1).
//!
//! capnp's default `ReaderOptions` allow 8 Mi words (64 MiB), and packing
//! turns a run of 256 zero words into two bytes, so a reader without a
//! bound lets one small datagram or DHT value exhaust memory. Every value
//! the decoders read comes from a peer, so the codec has one reader,
//! `unpack_bounded` in `crates/rekindle-codec/src/capnp_codec/mod.rs`, and
//! each decoder passes its type's bound from `capnp_codec::bounds`. This
//! gate rejects, in non-test code under `crates/*/src` and `src-tauri/src`:
//! - a capnp reader (`serialize::read_message*`, `try_read_message*`, and
//!   the `serialize_packed` ones) anywhere but that file;
//! - in that file, a function calling a reader without calling
//!   `traversal_limit_in_words`;
//! - in that file, `ReaderOptions` or a reader in any function but
//!   `unpack_bounded`;
//! - `traversal_limit_in_words` with any argument but
//!   `Some(<a parameter>.max_words)`, so `None`, `Option::None`, a variable
//!   and `Some(usize::MAX)` are refused;
//! - a call of `unpack_bounded` whose bound is not a `bounds::<NAME>` naming
//!   a `Bound` const declared in `capnp_codec/bounds.rs`, and `usize::MAX`
//!   anywhere in that file;
//! - outside that file, any `ReaderOptions` (every reader needs one, so
//!   this also catches a reader spelled through `message::Reader::new` and
//!   a `BufferSegments`/`SliceSegments`), `message::Reader::new`, an import
//!   of `capnp::message::Reader`, and a renamed or glob import of capnp's
//!   `serialize`, `serialize_packed` or `message` module or of `capnp`
//!   itself, any of which would let a reader hide behind another name.
//!
//! A gate on syntax cannot see inside a macro's token body, so the check
//! also reads the resolved graph from `cargo metadata`: only the packages
//! in [`CAPNP_DEPENDENTS`] may depend on the `capnp` crate at all, so no
//! other crate can call a reader however it spells it. A file that does not
//! parse is refused rather than skipped.
//!
//! Test code (`#[cfg(test)]`, `#[test]`, `tests/`, `tests.rs`,
//! `*_tests.rs`) builds its messages locally and is not scanned.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::walk_source_files;
use crate::wall_clock::{is_test_attr, is_test_file};

/// The one file allowed to call a capnp reader.
const READER_FILE: &str = "crates/rekindle-codec/src/capnp_codec/mod.rs";

/// The file that names every decoder's bound.
const BOUNDS_FILE: &str = "crates/rekindle-codec/src/capnp_codec/bounds.rs";

/// The packages that may depend on `capnp`, checked against `cargo
/// metadata`:
/// - `rekindle-codec`, the one Rekindle crate that encodes and decodes
///   Cap'n Proto;
/// - `capnpc`, the schema compiler `rekindle-codec`'s build script runs
///   (and veilid-core's), which depends on `capnp` for its own schema
///   reader;
/// - `veilid-core` (the pinned fork, ADR 0017), whose RPC wire is Cap'n
///   Proto; it decodes its own messages, never ours.
const CAPNP_DEPENDENTS: &[&str] = &["rekindle-codec", "capnpc", "veilid-core"];

/// Each package of `dependents` (those depending on `capnp`) that is not
/// allowed to.
fn capnp_dependent_hits(dependents: &BTreeSet<String>) -> Vec<String> {
    dependents
        .iter()
        .filter(|name| !CAPNP_DEPENDENTS.contains(&name.as_str()))
        .map(|name| format!("{name} depends on capnp (only rekindle-codec decodes Cap'n Proto)"))
        .collect()
}

/// The one function allowed to build `ReaderOptions` and call a reader.
const READER_FN: &str = "unpack_bounded";

/// The names of the `Bound` consts declared in `src` (non-test items).
fn bound_names(src: &str) -> BTreeSet<String> {
    let Ok(file) = syn::parse_file(src) else {
        return BTreeSet::new();
    };
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Const(c) if !c.attrs.iter().any(is_test_attr) => {
                let syn::Type::Path(ty) = &*c.ty else {
                    return None;
                };
                ty.path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "Bound")
                    .then(|| c.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

/// `Some(<param>.max_words)` for one of `params`.
fn is_bound_limit(arg: &syn::Expr, params: &[String]) -> bool {
    let syn::Expr::Call(call) = arg else {
        return false;
    };
    let syn::Expr::Path(func) = &*call.func else {
        return false;
    };
    if !func.path.is_ident("Some") || call.args.len() != 1 {
        return false;
    }
    let Some(syn::Expr::Field(field)) = call.args.first() else {
        return false;
    };
    let syn::Member::Named(member) = &field.member else {
        return false;
    };
    let syn::Expr::Path(base) = &*field.base else {
        return false;
    };
    member == "max_words"
        && base
            .path
            .get_ident()
            .is_some_and(|i| params.iter().any(|p| i == p))
}

/// `bounds::<NAME>` (through any prefix) naming one of `names`.
fn is_named_bound(arg: &syn::Expr, names: &BTreeSet<String>) -> bool {
    let syn::Expr::Path(p) = arg else {
        return false;
    };
    let segs: Vec<String> = p
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    segs.len() >= 2 && segs[segs.len() - 2] == "bounds" && names.contains(&segs[segs.len() - 1])
}

fn is_reader_name(name: &str) -> bool {
    name.starts_with("read_message") || name.starts_with("try_read_message")
}

/// capnp's modules a reader can be reached through.
fn is_capnp_reader_module(name: &str) -> bool {
    matches!(name, "serialize" | "serialize_packed" | "message")
}

/// `…::message::Reader::new`: a reader built from raw segments.
fn is_segments_reader(path: &syn::Path) -> bool {
    let names: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    names
        .windows(3)
        .any(|w| w[0] == "message" && w[1] == "Reader" && w[2] == "new")
}

fn is_reader_path(path: &syn::Path) -> bool {
    let through_serialize = path
        .segments
        .iter()
        .any(|s| s.ident == "serialize" || s.ident == "serialize_packed");
    through_serialize
        && path
            .segments
            .last()
            .is_some_and(|s| is_reader_name(&s.ident.to_string()))
}

struct Finder<'a> {
    rel: &'a str,
    reader_file: bool,
    /// Within the function being visited: (calls a reader, sets a limit).
    in_fn: Option<(bool, bool)>,
    /// The function being visited and its parameter names.
    fn_name: Option<String>,
    params: Vec<String>,
    /// The `Bound` consts of [`BOUNDS_FILE`].
    bounds: &'a BTreeSet<String>,
    hits: Vec<String>,
}

impl Finder<'_> {
    fn hit(&mut self, what: &str) {
        self.hits.push(format!("{}  {what}", self.rel));
    }

    fn in_reader_fn(&self) -> bool {
        self.reader_file && self.fn_name.as_deref() == Some(READER_FN)
    }

    fn reader_seen(&mut self) {
        if self.in_reader_fn() {
            if let Some(state) = self.in_fn.as_mut() {
                state.0 = true;
                return;
            }
        }
        self.hit("capnp reader outside unpack_bounded (use the codec's bounded decoders)");
    }

    /// A hit outside the reader file only.
    fn outside_reader(&mut self, what: &str) {
        if !self.reader_file {
            self.hit(what);
        }
    }

    /// A `use` naming a reader through a `serialize` or `serialize_packed`
    /// module, or hiding capnp's reader modules behind another name.
    /// `from` is the path so far.
    fn use_tree(&mut self, tree: &syn::UseTree, through_serialize: bool, from: &[String]) {
        let in_capnp = from.first().is_some_and(|f| f == "capnp");
        let module = from.last().map(String::as_str);
        let in_reader_module = in_capnp && module.is_some_and(is_capnp_reader_module);
        match tree {
            syn::UseTree::Path(p) => {
                let here = p.ident == "serialize" || p.ident == "serialize_packed";
                let mut next = from.to_vec();
                next.push(p.ident.to_string());
                self.use_tree(&p.tree, through_serialize || here, &next);
                return;
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    self.use_tree(t, through_serialize, from);
                }
                return;
            }
            syn::UseTree::Glob(_) if in_reader_module || (in_capnp && from.len() == 1) => {
                self.outside_reader("glob import of a capnp reader module");
            }
            syn::UseTree::Rename(r)
                if (from.is_empty() && r.ident == "capnp")
                    || (in_capnp && r.ident == "self" && from.len() == 1)
                    || (in_capnp
                        && from.len() == 1
                        && is_capnp_reader_module(&r.ident.to_string()))
                    || (in_reader_module && r.ident == "self") =>
            {
                self.outside_reader("renamed import of capnp or a capnp reader module");
            }
            _ => {}
        }
        let name = match tree {
            syn::UseTree::Name(n) => Some(&n.ident),
            syn::UseTree::Rename(r) => Some(&r.ident),
            _ => None,
        };
        if let Some(name) = name {
            if name == "ReaderOptions" {
                self.outside_reader("capnp ReaderOptions outside unpack_bounded");
            } else if in_capnp && module == Some("message") && name == "Reader" {
                self.outside_reader("import of capnp::message::Reader outside unpack_bounded");
            }
        }
        match tree {
            syn::UseTree::Name(n) if through_serialize && is_reader_name(&n.ident.to_string()) => {
                self.reader_seen();
            }
            syn::UseTree::Rename(r)
                if through_serialize && is_reader_name(&r.ident.to_string()) =>
            {
                self.reader_seen();
            }
            _ => {}
        }
    }

    fn visit_fn_body(&mut self, sig: &syn::Signature, body: impl FnOnce(&mut Self)) {
        let name = sig.ident.to_string();
        let params = sig
            .inputs
            .iter()
            .filter_map(|input| match input {
                syn::FnArg::Typed(t) => match &*t.pat {
                    syn::Pat::Ident(i) => Some(i.ident.to_string()),
                    _ => None,
                },
                syn::FnArg::Receiver(_) => None,
            })
            .collect();
        let outer = self.in_fn.replace((false, false));
        let outer_name = self.fn_name.replace(name.clone());
        let outer_params = std::mem::replace(&mut self.params, params);
        body(self);
        self.fn_name = outer_name;
        self.params = outer_params;
        if let Some((reads, limits)) = self.in_fn.take() {
            if reads && !limits {
                self.hit(&format!(
                    "fn {name} reads a capnp message without traversal_limit_in_words"
                ));
            }
        }
        self.in_fn = outer;
    }
}

impl<'ast> Visit<'ast> for Finder<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if !m.attrs.iter().any(is_test_attr) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if f.attrs.iter().any(is_test_attr) {
            return;
        }
        self.visit_fn_body(&f.sig, |s| {
            syn::visit::visit_item_fn(s, f);
        });
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if f.attrs.iter().any(is_test_attr) {
            return;
        }
        self.visit_fn_body(&f.sig, |s| {
            syn::visit::visit_impl_item_fn(s, f);
        });
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.use_tree(&item.tree, false, &[]);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        if item.ident == "capnp" && item.rename.is_some() {
            self.outside_reader("renamed import of capnp or a capnp reader module");
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if is_reader_path(path) {
            self.reader_seen();
        }
        if path.segments.iter().any(|s| s.ident == "ReaderOptions") && !self.in_reader_fn() {
            self.hit("capnp ReaderOptions outside unpack_bounded");
        }
        if self.rel == BOUNDS_FILE {
            let names: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
            if names.windows(2).any(|w| w[0] == "usize" && w[1] == "MAX") {
                self.hit("usize::MAX in a bound");
            }
        }
        if is_segments_reader(path) {
            self.outside_reader("capnp message::Reader::new outside unpack_bounded");
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let calls_reader = matches!(&*call.func, syn::Expr::Path(p)
            if p.path.segments.last().is_some_and(|s| s.ident == READER_FN));
        if calls_reader
            && !call
                .args
                .iter()
                .nth(1)
                .is_some_and(|b| is_named_bound(b, self.bounds))
        {
            self.hit("unpack_bounded takes a named Bound const from capnp_codec::bounds");
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "traversal_limit_in_words" {
            let bounded = call.args.len() == 1
                && call
                    .args
                    .first()
                    .is_some_and(|a| is_bound_limit(a, &self.params));
            if !bounded {
                self.hit(
                    "traversal_limit_in_words takes Some(<bound>.max_words), never None or a constant",
                );
            } else if let Some(state) = self.in_fn.as_mut() {
                state.1 = true;
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

#[cfg(test)]
fn scan(rel: &str, src: &str) -> Vec<String> {
    let bounds = ["SIGNED_ENVELOPE".to_owned()].into_iter().collect();
    scan_with(rel, src, &bounds)
}

fn scan_with(rel: &str, src: &str, bounds: &BTreeSet<String>) -> Vec<String> {
    let Ok(parsed) = syn::parse_file(src) else {
        return vec![format!("{rel}  does not parse, so it cannot be checked")];
    };
    let mut finder = Finder {
        rel,
        reader_file: rel == READER_FILE,
        in_fn: None,
        fn_name: None,
        params: Vec::new(),
        bounds,
        hits: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.hits
}

pub(crate) fn check_capnp_bounds(root: &Path) -> Result<()> {
    let mut dirs = vec![root.join("src-tauri/src")];
    for entry in std::fs::read_dir(root.join("crates"))? {
        dirs.push(entry?.path().join("src"));
    }
    let bounds = bound_names(&std::fs::read_to_string(root.join(BOUNDS_FILE))?);
    if bounds.is_empty() {
        return Err(anyhow!("{BOUNDS_FILE} declares no Bound const"));
    }
    let meta = crate::cargo_meta::Metadata::load(root)?;
    let mut hits = capnp_dependent_hits(&meta.direct_dependents("capnp"));
    for dir in dirs {
        for path in walk_source_files(&dir, &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if is_test_file(&rel) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            hits.extend(scan_with(&rel, &src, &bounds));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        eprintln!("    {h}");
    }
    Err(anyhow!(
        "{} unbounded Cap'n Proto read(s); decode through rekindle-codec with a bound",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{scan, READER_FILE};

    #[test]
    fn a_reader_outside_the_codec_is_refused() {
        let src = "fn f(b: &[u8]) { capnp::serialize_packed::read_message(b, \
                   capnp::message::ReaderOptions::new()); }";
        assert_eq!(
            scan("crates/x/src/lib.rs", src).len(),
            2,
            "the reader and its options"
        );
        let used = "use capnp::serialize::read_message_from_flat_slice;";
        assert_eq!(scan("crates/x/src/lib.rs", used).len(), 1);
        let grouped = "use capnp::{serialize_packed::{read_message as r}};";
        assert_eq!(scan("crates/x/src/lib.rs", grouped).len(), 1);
        let codec_fn = "use super::read::{read_message_edited};";
        assert!(scan("crates/x/src/lib.rs", codec_fn).is_empty());
    }

    /// The three spellings that once passed: an aliased module, a glob, and
    /// a reader built from segments.
    #[test]
    fn a_reader_behind_another_name_is_refused() {
        let lib = "crates/x/src/lib.rs";
        let alias = "use capnp::serialize_packed as sp; \
                     fn f(b: &[u8]) { sp::read_message(b, Default::default()); }";
        assert!(!scan(lib, alias).is_empty());
        let glob = "use capnp::serialize::*; \
                    fn f(b: &[u8]) { read_message(b, Default::default()); }";
        assert!(!scan(lib, glob).is_empty());
        let segments = "fn f(b: &[u8]) { capnp::message::Reader::new(\
                        capnp::serialize::BufferSegments::new(b, Default::default()).unwrap(), \
                        Default::default()); }";
        assert!(!scan(lib, segments).is_empty());
        let options = "use capnp::message::ReaderOptions as O; fn f() { O::new(); }";
        assert!(!scan(lib, options).is_empty());
        let reader = "use capnp::message::{Reader as R}; fn f() {}";
        assert!(!scan(lib, reader).is_empty());
        let crate_alias = "use capnp as c; fn f() {}";
        assert!(!scan(lib, crate_alias).is_empty());
        let extern_alias = "extern crate capnp as c;";
        assert!(!scan(lib, extern_alias).is_empty());
        let module_alias = "use capnp::{message as m};";
        assert!(!scan(lib, module_alias).is_empty());
        let plain = "use capnp::message; use crate::chat::message::*; \
                     fn f(b: &message::Builder<message::HeapAllocator>) {}";
        assert!(scan(lib, plain).is_empty());
    }

    #[test]
    fn the_codec_reader_needs_a_limit() {
        let unbounded = "fn unpack_bounded(b: &[u8]) { capnp::serialize_packed::read_message(b, \
                         capnp::message::ReaderOptions::new()); }";
        assert_eq!(scan(READER_FILE, unbounded).len(), 1);
        let bounded = "fn unpack_bounded(b: &[u8], bound: Bound) { \
                       let mut o = capnp::message::ReaderOptions::new(); \
                       o.traversal_limit_in_words(Some(bound.max_words)); \
                       capnp::serialize_packed::read_message(b, o); }";
        assert!(scan(READER_FILE, bounded).is_empty());
        for limit in [
            "Some(usize::MAX)",
            "Option::None",
            "Some(words)",
            "Some(other.max_words)",
            "limit",
        ] {
            let src = format!(
                "fn unpack_bounded(b: &[u8], bound: Bound) {{ \
                 let mut o = capnp::message::ReaderOptions::new(); \
                 o.traversal_limit_in_words({limit}); \
                 capnp::serialize_packed::read_message(b, o); }}"
            );
            assert!(!scan(READER_FILE, &src).is_empty(), "{limit}");
        }
        let elsewhere = "fn other(b: &[u8], bound: Bound) { \
                         let mut o = capnp::message::ReaderOptions::new(); \
                         o.traversal_limit_in_words(Some(bound.max_words)); \
                         capnp::serialize_packed::read_message(b, o); }";
        assert!(!scan(READER_FILE, elsewhere).is_empty());
        let none = "fn unpack_bounded() { let mut o = capnp::message::ReaderOptions::new(); \
                    o.traversal_limit_in_words(None); \
                    capnp::serialize_packed::read_message(b, o); }";
        assert_eq!(scan(READER_FILE, none).len(), 2);
    }

    #[test]
    fn every_call_names_a_bound_const() {
        let lib = "crates/rekindle-codec/src/x.rs";
        let named = "fn f(b: &[u8]) { unpack_bounded(b, bounds::SIGNED_ENVELOPE); }";
        assert!(scan(lib, named).is_empty());
        for bound in [
            "bounds::NOT_A_BOUND",
            "SIGNED_ENVELOPE",
            "Bound::packed(9)",
            "Bound { max_bytes: 1, max_words: usize::MAX }",
            "b",
        ] {
            let src = format!("fn f(b: &[u8]) {{ unpack_bounded(b, {bound}); }}");
            assert!(!scan(lib, &src).is_empty(), "{bound}");
        }
        let max = "pub const X: Bound = Bound { max_bytes: 1, max_words: usize::MAX };";
        assert!(!scan(super::BOUNDS_FILE, max).is_empty());
        let names =
            super::bound_names("pub const A: Bound = Bound::packed(1); pub const B: usize = 2;");
        assert_eq!(names.into_iter().collect::<Vec<_>>(), ["A"]);
    }

    #[test]
    fn only_the_codec_may_depend_on_capnp() {
        let allowed: BTreeSet<String> = ["rekindle-codec", "capnpc", "veilid-core"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert!(super::capnp_dependent_hits(&allowed).is_empty());
        let mut with_protocol = allowed;
        with_protocol.insert("rekindle-protocol".to_owned());
        assert_eq!(super::capnp_dependent_hits(&with_protocol).len(), 1);
    }

    #[test]
    fn a_file_that_does_not_parse_is_refused() {
        assert_eq!(scan("crates/x/src/lib.rs", "fn (").len(), 1);
    }

    #[test]
    fn test_code_is_not_scanned() {
        let src = "#[cfg(test)] mod tests { fn t(b: &[u8]) { \
                   capnp::serialize::read_message(b, Default::default()); } }";
        assert!(scan("crates/x/src/lib.rs", src).is_empty());
    }
}
