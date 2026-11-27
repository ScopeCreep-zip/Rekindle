//! Named bindings introduced by Rust use trees.

use std::collections::BTreeMap;

pub(crate) fn collect_uses(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut BTreeMap<String, Vec<String>>,
    glob: &mut impl FnMut(&[String]),
) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            collect_uses(&p.tree, prefix, out, glob);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            if n.ident == "self" {
                full.pop();
                if let Some(last) = full.last().cloned() {
                    out.insert(last, full);
                }
            } else {
                out.insert(n.ident.to_string(), full);
            }
        }
        syn::UseTree::Rename(r) => {
            let mut full = prefix.clone();
            if r.ident != "self" {
                full.push(r.ident.to_string());
            }
            out.insert(r.rename.to_string(), full);
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_uses(t, prefix, out, glob);
            }
        }
        syn::UseTree::Glob(_) => glob(prefix),
    }
}
