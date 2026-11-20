//! The Mumble front-end: a theme directory with `theme.ini` (`[theme]
//! styles=dark,lite`, then per style `qss=…`; `skins/raw/a/mumble-theme/
//! theme.ini:1-11`, `a-legacy-formats.md` §5.2), at the tree's root or
//! under one top-level folder. The first style's QSS gives the palette;
//! every PNG and SVG in the theme is an icon candidate.

use super::{ini, palette, root_of, text, Front, Read};
use crate::import::pidgin::check_limits;
use crate::import::pidgin::map::plain_path;
use crate::import::{ImportError, Outcome, Tree};

/// The `source.format` of an imported package.
pub const MUMBLE_FORMAT: &str = "mumble-theme";
const THEME_INI: &str = "theme.ini";

/// The INI value of `key` in `section`.
fn value<'a>(entries: &'a [(String, String, String)], section: &str, key: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|(s, k, _)| s == section && k == key)
        .map(|(.., v)| v.as_str())
}

/// Reads a Mumble theme tree.
///
/// # Errors
/// Past the legacy limits, no `theme.ini`, no style or `qss=`, a QSS that
/// is absent, not UTF-8 or has no `ApplicationPalette` block.
pub fn read(tree: &Tree) -> Result<Read, ImportError> {
    check_limits(tree)?;
    let root = root_of(tree, THEME_INI).ok_or_else(|| ImportError::MissingFile {
        file: THEME_INI.to_owned(),
    })?;
    let ini_path = format!("{root}{THEME_INI}");
    let entries = ini(text(tree, &ini_path)?);
    let bad = |detail: String| ImportError::Format {
        file: ini_path.clone(),
        detail,
    };
    let styles: Vec<&str> = value(&entries, "theme", "styles")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let style = *styles
        .first()
        .ok_or_else(|| bad("[theme] names no styles".to_owned()))?;
    let qss_name = value(&entries, style, "qss")
        .filter(|q| plain_path(q))
        .ok_or_else(|| bad(format!("[{style}] has no plain relative qss=")))?;
    let qss_path = format!("{root}{qss_name}");
    let qss = text(tree, &qss_path)?;
    let decls = palette::application_palette(qss).ok_or_else(|| ImportError::Format {
        file: qss_path.clone(),
        detail: "no ApplicationPalette block".to_owned(),
    })?;

    let mut read = Read::new(Front::Mumble);
    read.files.push(format!("{ini_path} style {style}"));
    read.files.push(qss_path.clone());
    for other in &styles[1..] {
        read.drop(
            format!("{ini_path} style {other}"),
            format!("dropped: one palette per pack; the first style, {style}, is imported"),
        );
    }
    for (key, v) in &decls {
        match key.strip_prefix("qproperty-") {
            Some(role) => read.colour(&qss_path, role, v),
            None => read.drop(format!("{qss_path} {key}"), "dropped: not a qproperty"),
        }
    }
    read.drop(
        format!("{qss_path} widget rules"),
        "dropped: QSS is converted to tokens, never applied",
    );
    super::background::read(tree, &mut read, &root, &qss_path, qss);
    let images = tree.iter().filter(|(k, _)| {
        k.starts_with(&root)
            && std::path::Path::new(k.as_str())
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("svg"))
    });
    let backgrounds: Vec<String> = read
        .regions
        .values()
        .flat_map(|images| images.values().map(|(.., source)| source.clone()))
        .collect();
    for (path, bytes) in images.filter(|(k, _)| !backgrounds.contains(k)) {
        read.image(path, bytes);
    }
    Ok(read)
}

/// Imports the Mumble theme tree as the package `id`: a passing package,
/// or a draft naming every failing pair.
///
/// # Errors
/// What [`read`] refuses.
pub fn import(tree: &Tree, id: &str) -> Result<Outcome, ImportError> {
    let read = read(tree)?;
    super::package(tree, &read, id)
}
