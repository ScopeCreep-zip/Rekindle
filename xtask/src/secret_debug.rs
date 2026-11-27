//! Gate hardening item 3(d). Inspect first-party Cargo targets, including
//! cfg-gated modules and derives. Vendored dependencies and generated
//! include/macro expansions are outside this source pass.
//!
//! A Debug implementation on a secret must leave the instance untouched.
//! Checking the receiver also covers format arguments, helpers and
//! destructuring, where looking for a particular formatter call would miss
//! the flow of secret data.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Result};
use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{Attribute, ImplItem, Item, Meta, Token, Type};

use crate::rust_uses::collect_uses;

#[path = "secret_debug_modules.rs"]
mod modules;
use modules::SourceModule;

type Symbol = Vec<String>;

#[derive(Default)]
struct SecretIndex {
    roots: BTreeSet<String>,
    imports: BTreeMap<Symbol, Symbol>,
    values: BTreeMap<Symbol, TypeShape>,
    zeroized: BTreeSet<Symbol>,
}

#[derive(Default)]
struct TypeShape {
    dependencies: BTreeSet<Symbol>,
    zeroizing_field: bool,
}

fn segments(path: &syn::Path) -> Symbol {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

fn child(module: &[String], name: &syn::Ident) -> Symbol {
    let mut path = module.to_vec();
    path.push(name.to_string());
    path
}

impl SecretIndex {
    fn absolute(&self, module: &[String], path: &[String]) -> Symbol {
        let mut base = module;
        let mut tail = path;
        match tail.split_first() {
            Some((first, rest)) if first == "crate" => {
                base = &module[..1];
                tail = rest;
            }
            Some((first, rest)) if first == "self" => tail = rest,
            Some((first, _)) if first == "super" => {
                while let Some((first, rest)) = tail.split_first() {
                    if first != "super" || base.len() == 1 {
                        break;
                    }
                    base = &base[..base.len() - 1];
                    tail = rest;
                }
            }
            Some((first, _)) if self.roots.contains(first) => return path.to_vec(),
            _ => {}
        }
        base.iter().chain(tail).cloned().collect()
    }

    fn resolve(&self, module: &[String], path: &[String]) -> Symbol {
        let mut resolved = self.absolute(module, path);
        let mut visited = BTreeSet::new();
        while let Some((key, dest)) = (2..=resolved.len())
            .rev()
            .find_map(|len| self.imports.get_key_value(&resolved[..len]))
        {
            if !visited.insert(key) {
                break;
            }
            resolved = dest.iter().chain(&resolved[key.len()..]).cloned().collect();
        }
        resolved
    }

    fn from_scopes(scopes: &[SourceModule]) -> Self {
        let mut index = Self::default();
        index
            .roots
            .extend(["core", "std", "zeroize"].map(String::from));
        for scope in scopes {
            index.roots.extend(scope.module.first().cloned());
        }
        index.record_imports(scopes);
        for scope in scopes {
            for item in &scope.items {
                index.record(&scope.module, item);
            }
        }
        index
    }

    fn record_imports(&mut self, scopes: &[SourceModule]) {
        let mut exports = BTreeMap::<Symbol, BTreeSet<String>>::new();
        for (module, names) in [
            (
                vec!["zeroize"],
                &["Zeroize", "ZeroizeOnDrop", "Zeroizing"][..],
            ),
            (vec!["core", "fmt"], &["Debug"][..]),
            (vec!["std", "fmt"], &["Debug"][..]),
        ] {
            exports.insert(
                module.into_iter().map(String::from).collect(),
                names.iter().map(|name| (*name).to_owned()).collect(),
            );
        }
        let mut globs = Vec::new();
        for scope in scopes {
            if let Some((name, parent)) = scope.module.split_last() {
                exports
                    .entry(parent.to_vec())
                    .or_default()
                    .insert(name.clone());
            }
            let mut uses = BTreeMap::new();
            for item in &scope.items {
                if let Item::Use(import) = item {
                    collect_uses(&import.tree, &mut Vec::new(), &mut uses, &mut |path| {
                        globs.push((&scope.module, path.to_vec()));
                    });
                } else if let Some((name, _)) = declaration(item) {
                    exports
                        .entry(scope.module.clone())
                        .or_default()
                        .insert(name.to_string());
                }
            }
            for (binding, dest) in uses {
                let resolved = self.absolute(&scope.module, &dest);
                let mut key = scope.module.clone();
                key.push(binding.clone());
                self.imports.insert(key, resolved);
                exports
                    .entry(scope.module.clone())
                    .or_default()
                    .insert(binding);
            }
        }
        loop {
            let mut changed = false;
            for (module, glob) in &globs {
                let source = self.resolve(module, glob);
                let Some(names) = exports.get(&source).cloned() else {
                    continue;
                };
                for name in names {
                    if exports
                        .entry((*module).clone())
                        .or_default()
                        .insert(name.clone())
                    {
                        let mut binding = (*module).clone();
                        binding.push(name.clone());
                        let mut dest = source.clone();
                        dest.push(name);
                        self.imports.insert(binding, dest);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn record(&mut self, module: &[String], item: &Item) {
        if let Item::Impl(item) = item {
            if let Some((None, tr, _)) = &item.trait_ {
                if zeroize_trait(&self.resolve(module, &segments(tr))) {
                    if let Some(ty) = self.impl_target(module, item) {
                        self.zeroized.insert(ty);
                    }
                }
            }
            return;
        }
        let Some((name, attrs)) = declaration(item) else {
            return;
        };
        let name = child(module, name);
        if derives(attrs).any(|path| zeroize_trait(&self.resolve(module, &segments(&path)))) {
            self.zeroized.insert(name.clone());
        }
        let mut value = StoredTypes {
            index: self,
            module,
            value: TypeShape::default(),
        };
        match item {
            Item::Struct(item) => value.visit_fields(&item.fields),
            Item::Enum(item) => item
                .variants
                .iter()
                .for_each(|variant| value.visit_fields(&variant.fields)),
            Item::Union(item) => value.visit_fields_named(&item.fields),
            Item::Type(item) => value.visit_type(&item.ty),
            _ => {}
        }
        let value = value.value;
        let known = self.values.entry(name).or_default();
        known.dependencies.extend(value.dependencies);
        known.zeroizing_field |= value.zeroizing_field;
    }

    fn impl_target(&self, module: &[String], item: &syn::ItemImpl) -> Option<Symbol> {
        let Type::Path(ty) = &*item.self_ty else {
            return None;
        };
        ty.qself
            .is_none()
            .then(|| self.resolve(module, &segments(&ty.path)))
    }

    fn secrets(&self) -> BTreeSet<&Symbol> {
        let mut wrapped: BTreeSet<_> = self
            .values
            .iter()
            .filter(|(name, value)| {
                name.first().is_some_and(|root| root == "rekindle_secrets")
                    && (value.zeroizing_field || self.zeroized.contains(*name))
            })
            .map(|(name, _)| name)
            .collect();
        loop {
            let next: Vec<_> = self
                .values
                .iter()
                .filter(|(name, value)| {
                    !wrapped.contains(*name)
                        && value.dependencies.iter().any(|dep| wrapped.contains(dep))
                })
                .map(|(name, _)| name)
                .collect();
            if next.is_empty() {
                break;
            }
            wrapped.extend(next);
        }
        wrapped.extend(&self.zeroized);
        wrapped
    }
}

struct StoredTypes<'a> {
    index: &'a SecretIndex,
    module: &'a [String],
    value: TypeShape,
}

impl<'ast> Visit<'ast> for StoredTypes<'_> {
    fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
        if ty.qself.is_none() {
            let path = self.index.resolve(self.module, &segments(&ty.path));
            if path == ["zeroize", "Zeroizing"] {
                self.value.zeroizing_field = true;
            }
            if path == ["std", "marker", "PhantomData"] || path == ["core", "marker", "PhantomData"]
            {
                return;
            }
            self.value.dependencies.insert(path);
        }
        visit::visit_type_path(self, ty);
    }

    fn visit_type_bare_fn(&mut self, _ty: &'ast syn::TypeBareFn) {}
}

fn declaration(item: &Item) -> Option<(&syn::Ident, &[Attribute])> {
    match item {
        Item::Struct(item) => Some((&item.ident, &item.attrs)),
        Item::Enum(item) => Some((&item.ident, &item.attrs)),
        Item::Union(item) => Some((&item.ident, &item.attrs)),
        Item::Type(item) => Some((&item.ident, &item.attrs)),
        _ => None,
    }
}

fn zeroize_trait(path: &[String]) -> bool {
    path == ["zeroize", "Zeroize"] || path == ["zeroize", "ZeroizeOnDrop"]
}

fn debug_trait(path: &[String]) -> bool {
    path == ["core", "fmt", "Debug"] || path == ["std", "fmt", "Debug"]
}

fn derives(attrs: &[Attribute]) -> impl Iterator<Item = syn::Path> + '_ {
    let mut traits = Vec::new();
    for attr in attrs {
        collect_derives(&attr.meta, &mut traits);
    }
    traits.into_iter()
}

fn collect_derives(meta: &Meta, traits: &mut Vec<syn::Path>) {
    let Meta::List(list) = meta else { return };
    if list.path.is_ident("derive") {
        if let Ok(paths) =
            list.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
        {
            traits.extend(paths);
        }
    } else if list.path.is_ident("cfg_attr") {
        if let Ok(metas) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) {
            for attr in metas.iter().skip(1) {
                collect_derives(attr, traits);
            }
        }
    }
}

fn formats_receiver(tokens: TokenStream) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == "self",
        TokenTree::Group(group) => formats_receiver(group.stream()),
        TokenTree::Literal(literal) => syn::parse2::<syn::LitStr>(literal.into_token_stream())
            .is_ok_and(|text| {
                text.value()
                    .split('{')
                    .skip(1)
                    .any(|capture| capture.starts_with("self}") || capture.starts_with("self:"))
            }),
        TokenTree::Punct(_) => false,
    })
}

fn finding(
    index: &SecretIndex,
    module: &[String],
    item: &Item,
    secrets: &BTreeSet<&Symbol>,
) -> Option<(Symbol, &'static str)> {
    if let Item::Impl(item) = item {
        let (None, tr, _) = item.trait_.as_ref()? else {
            return None;
        };
        let target = index.impl_target(module, item)?;
        return (secrets.contains(&target)
            && (tr.is_ident("Debug") || debug_trait(&index.resolve(module, &segments(tr))))
            && item.items.iter().any(|member| {
                matches!(member, ImplItem::Fn(method) if method.sig.ident == "fmt"
                        && formats_receiver(method.block.to_token_stream()))
            }))
        .then_some((target, "formats instance data"));
    }
    let (ident, attrs) = declaration(item)?;
    let name = child(module, ident);
    (secrets.contains(&name)
        && derives(attrs).any(|path| {
            path.is_ident("Debug") || debug_trait(&index.resolve(module, &segments(&path)))
        }))
    .then_some((name, "derives Debug"))
}

fn findings(scopes: &[SourceModule]) -> BTreeSet<String> {
    let index = SecretIndex::from_scopes(scopes);
    let secrets = index.secrets();
    scopes
        .iter()
        .flat_map(|scope| {
            scope.items.iter().filter_map(|item| {
                let (name, kind) = finding(&index, &scope.module, item, &secrets)?;
                Some(format!("{}: {} {kind}", scope.file.display(), name.last()?))
            })
        })
        .collect()
}

pub(crate) fn check(root: &Path) -> Result<()> {
    let mut scopes = modules::first_party_scopes(root)?;
    for scope in &mut scopes {
        scope.file = scope.file.strip_prefix(root)?.to_owned();
    }
    let hits = findings(&scopes);
    for hit in &hits {
        eprintln!("    ✗ {hit}");
    }
    if !hits.is_empty() {
        bail!(
            "{} secret Debug violation(s): print only the type name and <redacted>",
            hits.len()
        );
    }
    println!("    secret Debug implementations keep instance data redacted");
    Ok(())
}

#[cfg(test)]
#[path = "secret_debug_tests.rs"]
mod tests;
