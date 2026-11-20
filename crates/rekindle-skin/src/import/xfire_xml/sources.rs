//! The XML tree's included component files and the counts of what the
//! import drops (moved from `mod.rs`, plan SK0 S1, to keep it under the
//! file-size rule).

use std::collections::BTreeMap;

use super::super::tree::Node;
use super::super::xfire_ini::lookup;
use super::super::{ImportError, Tree};
use super::{descendants, ends_with_ci, v3, xml};

pub(super) fn count_dropped(tree: &Tree, components: &[Node]) -> BTreeMap<&'static str, usize> {
    let mut d = BTreeMap::new();
    let count = |name: &str| components.iter().map(|c| descendants(c, name).len()).sum();
    d.insert("triggers", count("Trigger"));
    d.insert("layout components", count("Component"));
    d.insert("layout tiles", count("Tile"));
    let html = tree
        .keys()
        .filter(|k| {
            std::path::Path::new(k.as_str())
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
        })
        .count();
    d.insert("HTML templates", html);
    d
}

/// Every included component file: Symbiosis's `Components/*.xml`,
/// Classic v3's [`v3::SOURCE`], and any other included file whose root is
/// `<Components>` (Onyx, Space Cowboy, StealthX and Total Chaos keep one
/// `components.xml` beside `skin.xml`; xSLIMFIRE `xSLIMFIREcomponents.xml`).
pub(super) fn component_files(tree: &Tree, included: &[String]) -> Result<Vec<Node>, ImportError> {
    let mut components = Vec::new();
    for f in included {
        if lookup(tree, f).is_err() {
            continue;
        }
        let named = f
            .get(..11)
            .is_some_and(|p| p.eq_ignore_ascii_case("Components/"))
            || f.eq_ignore_ascii_case(v3::SOURCE);
        if named || ends_with_ci(f, ".xml") {
            let root = xml(tree, f)?;
            if named || root.name.eq_ignore_ascii_case("Components") {
                components.push(root);
            }
        }
    }
    Ok(components)
}
