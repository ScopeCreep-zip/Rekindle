//! Legacy skin importers (plan E7.7.3): each reads a legacy tree held in
//! memory and builds a `rekindle-skin/1` package that then goes through
//! [`crate::validate::run`] like any other skin. Legacy archives are read
//! only here; nothing of them reaches a frontend but the canonical package.
//!
//! * [`xfire_xml`]: the Xfire XML era (Carbon Fiber, Symbiosis).
//! * [`xfire_ini`]: the Xfire INI era (DoiDD), a delta on `xfire_xml`.
//! * [`winamp`]: Winamp classic `.wsz` (a "Winamp-flavoured Rekindle skin").
//! * [`pidgin`]: Pidgin 2.x themes (buddy-list palette, status icons, sound
//!   events).
//! * [`mumble`] and [`teamspeak`]: Mumble themes and TeamSpeak 3 styles,
//!   one Qt-palette reader ([`qt`]) with two front-ends; SVG icons are
//!   rasterised with every reference refused.
//! * [`miranda`]: Miranda NG TabSRMM `.tsk` and Clist_modern `.msf` skins
//!   (colours and 9-slice images).
//! * [`walk`]: the minimal Oklab-lightness walk a built-in that fails the
//!   validator is adjusted by, every change recorded.
//!
//! The crate does no file I/O: the caller (`cargo xtask import-skin`, the
//! helper) hands the tree over as a map of relative paths to bytes.

pub mod miranda;
mod notice;
pub mod pidgin;
pub mod qt;
mod raster;
mod tree;
pub mod walk;
pub mod winamp;
pub mod xfire_ini;
pub mod xfire_xml;

pub use qt::{mumble, teamspeak};

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinBase, SkinError, TokenValue};

use crate::colour;
use crate::output::Canonical;
use crate::registry::Base;

/// A legacy tree: relative `/`-separated paths to file bytes.
pub type Tree = BTreeMap<String, Vec<u8>>;

/// Why an import stopped.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{file}: XML: {detail}")]
    Xml { file: String, detail: String },
    #[error("{file}: {detail}")]
    Format { file: String, detail: String },
    #[error("{file}: missing from the tree")]
    MissingFile { file: String },
    #[error("{name}: RGBA alpha {alpha} is not 0 (the byte is unused, COLORREF-style)")]
    NonZeroAlpha { name: String, alpha: i32 },
    #[error("{name}: {value} is not RGBA(r,g,b,a) with r, g, b in 0-255")]
    BadColour { name: String, value: String },
    #[error("{entry}: legacy archive refused: {detail}")]
    Archive { entry: String, detail: String },
    #[error("{file}: image: {detail}")]
    Image { file: String, detail: String },
    #[error("the package is refused: {0}")]
    Refused(SkinError),
    /// A built-in the walk cannot settle (a user import is a draft).
    #[error("the walk cannot settle {error}: {why}")]
    Unsettled { error: SkinError, why: String },
}

/// An imported skin: the canonical package and the `NOTICE` beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub canonical: Canonical,
    pub notice: String,
    /// The walk's changes, in the order they were made.
    pub changes: Vec<walk::Change>,
}

/// The BLAKE3 of a tree, as the manifest's `source.hash`: every file in
/// path order, each as its path length, path, byte length and bytes.
#[must_use]
pub fn tree_hash(tree: &Tree) -> String {
    let mut h = blake3::Hasher::new();
    for (path, bytes) in tree {
        h.update(&(path.len() as u64).to_le_bytes());
        h.update(path.as_bytes());
        h.update(&(bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    h.finalize().to_hex().to_string()
}

/// A user import that fails the validator.
#[derive(Debug)]
pub struct ImportDraft {
    /// The package as imported (a zip that fails [`crate::validate::run`]).
    pub package: Vec<u8>,
    /// Every failing pair the walk settled, with its suggested passing colour.
    pub failures: Vec<walk::Change>,
    /// The refusal the walk cannot settle, with no suggestion.
    pub unsettled: Option<walk::Unsettled>,
    /// The package with every suggestion accepted; `None` when a refusal is
    /// unsettled.
    pub suggested: Option<Imported>,
}

/// A passing import or a draft.
#[derive(Debug)]
pub enum Outcome {
    Passed(Imported),
    Draft(Box<ImportDraft>),
}

/// The base an imported palette is validated on: light when `bg.panel` is
/// light (relative luminance above 0.179), else dark.
pub(crate) fn base_of(tokens: &BTreeMap<String, TokenValue>) -> (SkinBase, Base) {
    let ground = tokens.get("bg.panel").and_then(|v| match v {
        TokenValue::Color { r, g, b, .. } => Some([*r, *g, *b]),
        _ => None,
    });
    if ground.is_some_and(|g| colour::luminance(g) > 0.179) {
        (SkinBase::Light, Base::Light)
    } else {
        (SkinBase::Dark, Base::Dark)
    }
}

/// Builds the package from `tokens`, walks it until it passes, and returns
/// a passing import, or a draft naming every failing pair (with every
/// suggestion accepted, or the refusal the walk cannot settle).
///
/// # Errors
/// What `build` returns.
pub(crate) fn conclude(
    mut tokens: BTreeMap<String, TokenValue>,
    base: Base,
    build: impl Fn(&BTreeMap<String, TokenValue>) -> Result<Vec<u8>, ImportError>,
    notice: impl FnOnce(&[walk::Change]) -> String,
) -> Result<Outcome, ImportError> {
    let package = build(&tokens)?;
    let (settled, changes) = walk::settle(&mut tokens, base, build)?;
    let canonical = match settled {
        walk::WalkResult::Passed(c) => c,
        walk::WalkResult::Unsettled(u) => {
            return Ok(Outcome::Draft(Box::new(ImportDraft {
                package,
                failures: changes,
                unsettled: Some(u),
                suggested: None,
            })));
        }
    };
    let imported = Imported {
        notice: notice(&changes),
        canonical,
        changes,
    };
    if imported.changes.is_empty() {
        return Ok(Outcome::Passed(imported));
    }
    Ok(Outcome::Draft(Box::new(ImportDraft {
        package,
        failures: imported.changes.clone(),
        unsettled: None,
        suggested: Some(imported),
    })))
}
