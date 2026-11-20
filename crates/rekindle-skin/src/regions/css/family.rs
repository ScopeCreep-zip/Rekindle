//! The region sheets split by slot family (plan E7.7.7): one components
//! and one a11y sheet per window or control, generated under
//! `src/styles/generated/regions/` and imported in slot order by the two
//! index sheets `global.css` imports.

use std::collections::BTreeMap;

use super::{a11y_sheet, rules, RegionMap};
use crate::regions::slots::SLOTS;

/// A slot's family: the window or control it belongs to, its id's first
/// segment (`buddylist.tab` → `buddylist`). The generated sheets are one
/// file per family (`generated/regions/<family>.css`, `<family>-a11y.css`),
/// so no sheet outgrows the 600-line ceiling as windows gain slots.
#[must_use]
pub fn family(id: &str) -> &str {
    id.split('.').next().unwrap_or(id)
}

/// The painted families, in slot order (the cascade order of the files).
#[must_use]
pub fn families() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for slot in SLOTS.iter().filter(|s| s.css) {
        let f = family(slot.id);
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

/// One family's two sheets, `(components, a11y)`: what [`app_css`] and
/// [`app_a11y_css`] hold for the slots of `family` alone.
///
/// # Errors
/// What [`app_css`] returns.
pub fn family_css(
    header: &str,
    map: &BTreeMap<String, RegionMap>,
    family: &str,
) -> Result<(String, String), String> {
    let (components, a11y) = rules(map, Some(family))?;
    Ok((
        format!(
            "/* {header}\n *\n * The `{family}` region slots (`rekindle-skin::regions`) on the\n * elements `src/styles/regions.toml` `[regions]` names. */\n@layer components {{\n{components}}}\n"
        ),
        a11y_sheet(header, &a11y),
    ))
}
