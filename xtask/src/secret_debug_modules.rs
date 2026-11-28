//! Module paths for the secret Debug check, including file-backed test modules.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use syn::{Expr, Item, Lit, Meta};

use crate::cargo_meta::Metadata;

pub(super) struct SourceModule {
    pub(super) module: Vec<String>,
    pub(super) file: PathBuf,
    pub(super) items: Vec<Item>,
}

pub(super) fn first_party_scopes(root: &Path) -> Result<Vec<SourceModule>> {
    let meta = Metadata::load(root)?;
    let mut scopes = Vec::new();
    for package in meta.members() {
        if package
            .manifest_path
            .starts_with(root.join("crates/vendor"))
        {
            continue;
        }
        for target in &package.targets {
            // Only libraries can be imported by other crates. Give the other
            // targets distinct roots, even when Cargo gives them the same name.
            let namespace = if target
                .kind
                .iter()
                .any(|kind| matches!(kind.as_str(), "lib" | "rlib" | "dylib" | "proc-macro"))
            {
                target.name.clone()
            } else {
                target.src_path.display().to_string()
            };
            let directory = parent(&target.src_path)?;
            load(&target.src_path, directory, vec![namespace], &mut scopes)?;
        }
    }
    Ok(scopes)
}

fn parent(file: &Path) -> Result<&Path> {
    file.parent()
        .with_context(|| format!("no parent directory for {}", file.display()))
}

fn load(
    file: &Path,
    directory: &Path,
    module: Vec<String>,
    scopes: &mut Vec<SourceModule>,
) -> Result<()> {
    let src =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let parsed = syn::parse_file(&src).with_context(|| format!("parsing {}", file.display()))?;
    collect(parsed.items, file, directory, module, scopes)
}

fn collect(
    items: Vec<Item>,
    file: &Path,
    directory: &Path,
    module: Vec<String>,
    scopes: &mut Vec<SourceModule>,
) -> Result<()> {
    let mut local = Vec::new();
    for item in items {
        let Item::Mod(mut child) = item else {
            local.push(item);
            continue;
        };
        let name = child.ident.to_string();
        let mut child_module = module.clone();
        child_module.push(name.clone());
        let nested = directory.join(&name);
        if let Some((_, items)) = child.content.take() {
            collect(items, file, &nested, child_module, scopes)?;
        } else if let Some(path) = child.attrs.iter().find_map(module_path) {
            let child_file = parent(file)?.join(path);
            load(&child_file, parent(&child_file)?, child_module, scopes)?;
        } else {
            let flat_file = directory.join(format!("{name}.rs"));
            let child_file = if flat_file.is_file() {
                flat_file
            } else {
                nested.join("mod.rs")
            };
            load(&child_file, &nested, child_module, scopes)?;
        }
    }
    scopes.push(SourceModule {
        module,
        file: file.to_owned(),
        items: local,
    });
    Ok(())
}

fn module_path(attr: &syn::Attribute) -> Option<String> {
    let Meta::NameValue(meta) = &attr.meta else {
        return None;
    };
    let Expr::Lit(expr) = &meta.value else {
        return None;
    };
    let Lit::Str(path) = &expr.lit else {
        return None;
    };
    meta.path.is_ident("path").then(|| path.value())
}
