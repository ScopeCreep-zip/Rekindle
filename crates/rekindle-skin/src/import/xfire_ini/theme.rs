//! A theme file's `[default]` section as the INI importer reads it: its
//! `ColorGroup.<NAME>=` colours, the tokens they set, and its
//! `Graphics.<ALIAS>` image aliases.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::TokenValue;

use super::super::xfire_xml::colours::{absolute, target_in, Target};
use super::super::xfire_xml::ColourRow;
use super::super::{walk, ImportError};
use super::ini::Section;
use super::{colours, rgba};

pub(super) type Theme = (
    Vec<ColourRow>,
    BTreeMap<String, TokenValue>,
    BTreeMap<String, String>,
);

/// The theme's colours, the tokens they set, and its `Graphics.<ALIAS>`
/// image aliases; every other key counted in `dropped`.
pub(super) fn read_theme(
    theme: &Section,
    dropped: &mut BTreeMap<&'static str, usize>,
    notes: &mut Vec<String>,
) -> Result<Theme, ImportError> {
    let (mut rows, mut tokens, mut aliases) = (Vec::new(), BTreeMap::new(), BTreeMap::new());
    for e in &theme.entries {
        if let Some(name) = e.key.strip_prefix("ColorGroup.") {
            let value = rgba::read(name, &e.value, notes)?;
            let t = target_in(colours::NAMES, name);
            if let Target::Token(id) = t {
                tokens.insert(id.to_owned(), walk::color(absolute(name, &e.value, value)?));
            }
            rows.push(ColourRow {
                name: name.to_owned(),
                value,
                target: t,
            });
        } else if let Some(alias) = e.key.strip_prefix("Graphics.") {
            aliases.insert(alias.to_ascii_uppercase(), e.value.clone());
        } else {
            *dropped.entry("theme keys (sounds, other)").or_default() += 1;
        }
    }
    Ok((rows, tokens, aliases))
}
