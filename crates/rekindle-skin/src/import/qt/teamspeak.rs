//! The TeamSpeak 3 front-end: a style tree (`.ts3_style` unpacked, or
//! the installed `styles/` folder; `a-legacy-formats.md` §5.1) holding
//! `styles/<name>.qss`, its variants (`_mac`, `_linux`, `_darkmode`,
//! `_chat`, …), `styles/<name>/media/*.png` and `*.tpl` HTML templates,
//! at the tree's root or under one top-level folder. The base stylesheet's
//! `QPalette::` overrides give the palette; the direct `media/` files are
//! icon candidates.

use super::{ini, palette, text, top_level, Front, Read};
use crate::import::pidgin::check_limits;
use crate::import::{ImportError, Outcome, Tree};

/// The `source.format` of an imported package.
pub const TEAMSPEAK_FORMAT: &str = "teamspeak3-style";
/// The stylesheet suffixes TS3 loads in addition to the base one.
const VARIANTS: [&str; 4] = ["_mac", "_linux", "_darkmode", "_chat"];

/// The folder holding `styles/`: the root, or one top-level folder.
fn styles_root(tree: &Tree) -> Option<String> {
    tree.keys().find_map(|k| {
        let (at, _) = k.split_once("styles/")?;
        top_level(at).then(|| at.to_owned())
    })
}

/// Every `styles/<stem>.qss` directly under `styles/`, in path order.
fn stylesheets<'t>(tree: &'t Tree, styles: &str) -> Vec<(&'t str, &'t str)> {
    tree.keys()
        .filter_map(|k| {
            let name = k.strip_prefix(styles)?.strip_suffix(".qss")?;
            (!name.contains('/')).then_some((k.as_str(), name))
        })
        .collect()
}

/// Reads a TeamSpeak 3 style tree.
///
/// # Errors
/// Past the legacy limits, no `styles/<name>.qss` base stylesheet, or one
/// that is not UTF-8.
pub fn read(tree: &Tree) -> Result<Read, ImportError> {
    check_limits(tree)?;
    let missing = || ImportError::MissingFile {
        file: "styles/<name>.qss".to_owned(),
    };
    let root = styles_root(tree).ok_or_else(missing)?;
    let styles = format!("{root}styles/");
    let sheets = stylesheets(tree, &styles);
    let is_variant = |name: &str| VARIANTS.iter().any(|v| name.contains(v));
    let (qss_path, stem) = *sheets
        .iter()
        .find(|(_, n)| !is_variant(n))
        .ok_or_else(missing)?;
    let qss = text(tree, qss_path)?;

    let mut read = Read::new(Front::TeamSpeak3);
    if let Some(package) = tree.get(&format!("{root}package.ini")) {
        let name = std::str::from_utf8(package).ok().and_then(|t| {
            ini(t)
                .into_iter()
                .find_map(|(_, k, v)| (k == "Name").then_some(v))
        });
        read.files.push(format!(
            "{root}package.ini Name = {}",
            name.unwrap_or_default()
        ));
    }
    read.files.push(qss_path.to_owned());
    for (path, name) in &sheets {
        if *path != qss_path {
            let why = if is_variant(name) {
                "dropped: a platform, dark-mode or chat variant; one palette per pack".to_owned()
            } else {
                format!("dropped: one style per pack; {stem} is imported")
            };
            read.drop(*path, why);
        }
    }
    for (role, value) in palette::overrides(qss, "QPalette::") {
        read.colour(qss_path, role, value);
    }
    for (name, value) in palette::overrides(qss, "CustomColor::") {
        read.drop(
            format!("{qss_path} CustomColor::{name} = {value}"),
            "dropped: no friend, blocked or recording row role",
        );
    }
    read.drop(
        format!("{qss_path} widget rules"),
        "dropped: QSS is converted to tokens, never applied",
    );
    let media = format!("{styles}{stem}/media/");
    for (path, bytes) in tree.range(media.clone()..) {
        let Some(name) = path.strip_prefix(&media) else {
            break;
        };
        if name.contains('/') {
            read.drop(path, "dropped: not a direct media/ file");
        } else {
            read.image(path, bytes);
        }
    }
    let template = |k: &&String| {
        std::path::Path::new(k.as_str())
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tpl"))
    };
    for path in tree.keys().filter(template) {
        read.drop(path, "dropped: an HTML info template; never rendered");
    }
    Ok(read)
}

/// Imports the TeamSpeak 3 style tree as the package `id`: a passing
/// package, or a draft naming every failing pair.
///
/// # Errors
/// What [`read`] refuses.
pub fn import(tree: &Tree, id: &str) -> Result<Outcome, ImportError> {
    let read = read(tree)?;
    super::package(tree, &read, id)
}
