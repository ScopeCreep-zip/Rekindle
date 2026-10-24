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
//! - `traversal_limit_in_words(None)` anywhere.
//!
//! Test code (`#[cfg(test)]`, `#[test]`, `tests/`, `tests.rs`,
//! `*_tests.rs`) builds its messages locally and is not scanned.

use std::path::Path;

use anyhow::{anyhow, Result};
use syn::visit::Visit;

use crate::walk_source_files;
use crate::wall_clock::{is_test_attr, is_test_file};

/// The one file allowed to call a capnp reader.
const READER_FILE: &str = "crates/rekindle-codec/src/capnp_codec/mod.rs";

fn is_reader_name(name: &str) -> bool {
    name.starts_with("read_message") || name.starts_with("try_read_message")
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
    hits: Vec<String>,
}

impl Finder<'_> {
    fn hit(&mut self, what: &str) {
        self.hits.push(format!("{}  {what}", self.rel));
    }

    fn reader_seen(&mut self) {
        if self.reader_file {
            if let Some(state) = self.in_fn.as_mut() {
                state.0 = true;
                return;
            }
        }
        self.hit("capnp reader outside unpack_bounded (use the codec's bounded decoders)");
    }

    /// A `use` naming a reader through a `serialize` or `serialize_packed`
    /// module.
    fn use_tree(&mut self, tree: &syn::UseTree, through_serialize: bool) {
        match tree {
            syn::UseTree::Path(p) => {
                let here = p.ident == "serialize" || p.ident == "serialize_packed";
                self.use_tree(&p.tree, through_serialize || here);
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    self.use_tree(t, through_serialize);
                }
            }
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

    fn visit_fn_body(&mut self, name: &str, body: impl FnOnce(&mut Self)) {
        let outer = self.in_fn.replace((false, false));
        body(self);
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
        self.visit_fn_body(&f.sig.ident.to_string(), |s| {
            syn::visit::visit_item_fn(s, f);
        });
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if f.attrs.iter().any(is_test_attr) {
            return;
        }
        self.visit_fn_body(&f.sig.ident.to_string(), |s| {
            syn::visit::visit_impl_item_fn(s, f);
        });
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.use_tree(&item.tree, false);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if is_reader_path(path) {
            self.reader_seen();
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "traversal_limit_in_words" {
            let unbounded = call
                .args
                .iter()
                .any(|a| matches!(a, syn::Expr::Path(p) if p.path.is_ident("None")));
            if unbounded {
                self.hit("traversal_limit_in_words(None) removes the bound");
            } else if let Some(state) = self.in_fn.as_mut() {
                state.1 = true;
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn scan(rel: &str, src: &str) -> Vec<String> {
    // A file that does not parse is cargo's problem to report.
    let Ok(parsed) = syn::parse_file(src) else {
        return Vec::new();
    };
    let mut finder = Finder {
        rel,
        reader_file: rel == READER_FILE,
        in_fn: None,
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
    let mut hits = Vec::new();
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
            hits.extend(scan(&rel, &src));
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
    use super::{scan, READER_FILE};

    #[test]
    fn a_reader_outside_the_codec_is_refused() {
        let src = "fn f(b: &[u8]) { capnp::serialize_packed::read_message(b, \
                   capnp::message::ReaderOptions::new()); }";
        assert_eq!(scan("crates/x/src/lib.rs", src).len(), 1);
        let used = "use capnp::serialize::read_message_from_flat_slice;";
        assert_eq!(scan("crates/x/src/lib.rs", used).len(), 1);
        let grouped = "use capnp::{serialize_packed::{read_message as r}};";
        assert_eq!(scan("crates/x/src/lib.rs", grouped).len(), 1);
        let codec_fn = "use super::read::{read_message_edited};";
        assert!(scan("crates/x/src/lib.rs", codec_fn).is_empty());
    }

    #[test]
    fn the_codec_reader_needs_a_limit() {
        let unbounded = "fn unpack(b: &[u8]) { capnp::serialize_packed::read_message(b, \
                         capnp::message::ReaderOptions::new()); }";
        assert_eq!(scan(READER_FILE, unbounded).len(), 1);
        let bounded = "fn unpack_bounded(b: &[u8], w: usize) { \
                       let mut o = capnp::message::ReaderOptions::new(); \
                       o.traversal_limit_in_words(Some(w)); \
                       capnp::serialize_packed::read_message(b, o); }";
        assert!(scan(READER_FILE, bounded).is_empty());
        let none = "fn f() { let mut o = capnp::message::ReaderOptions::new(); \
                    o.traversal_limit_in_words(None); \
                    capnp::serialize_packed::read_message(b, o); }";
        assert_eq!(scan(READER_FILE, none).len(), 2);
    }

    #[test]
    fn test_code_is_not_scanned() {
        let src = "#[cfg(test)] mod tests { fn t(b: &[u8]) { \
                   capnp::serialize::read_message(b, Default::default()); } }";
        assert!(scan("crates/x/src/lib.rs", src).is_empty());
    }
}
