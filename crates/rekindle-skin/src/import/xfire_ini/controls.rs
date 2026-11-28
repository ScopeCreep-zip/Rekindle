//! The painted control slots of an INI-era skin, by the XML era's rules
//! ([`super::super::xfire_xml::controls`]): a section named for a title
//! button (`[CloseBox]`, `[MinimizeBox]`, `[MinimizeButton]` …) gives its
//! `<State>.middle`; the first `Class=Scrollbar` section (Alloy's and
//! Simple X's `[DefaultScrollbar]`, XGlow's `[XglowScrollbar]`) gives its
//! `<State>.Scroll*` pieces. A state is a key prefix ([`STATES`]:
//! `Graphics` normal, `Hover`, `BtnDown` down, `Disabled` disabled, the
//! push button's own states); each piece is shifted by its prefix's
//! `ColorGroup` (foreground) or `ColorGroupBG` (background), as the frame's
//! pieces are by their section's `<State>.ColorGroup`.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::RegionState;

use super::super::xfire_xml::controls::{fill, is_read_piece, StateImages, StateTally};
use super::super::xfire_xml::surfaces::{self, Window};
use super::super::xfire_xml::{bitmap, ColourRow};
use super::super::{Control, ImportError, Tree};
use super::ini::{Ini, Section};
use super::resolve;

/// A control's state prefixes and the region state each becomes.
pub const STATES: [(&str, RegionState); 4] = [
    ("Graphics", RegionState::Normal),
    ("Hover", RegionState::Hover),
    ("BtnDown", RegionState::Down),
    ("Disabled", RegionState::Disabled),
];

/// What a skin's images are read through.
pub struct Images<'a> {
    pub tree: &'a Tree,
    pub colours: &'a [ColourRow],
    /// The theme's `Graphics.<NAME>=` aliases, upper-cased names.
    pub aliases: &'a BTreeMap<String, String>,
    /// Every named image the tree lacks (spec A8: it lays nothing).
    pub absent: RefCell<BTreeSet<String>>,
    /// Every file decoded, for the `NOTICE`'s unread list.
    pub files: RefCell<BTreeSet<String>>,
}

/// One prefix's keys of a control section.
struct IniSet<'a> {
    images: &'a Images<'a>,
    section: &'a Section,
    prefix: &'static str,
    state: RegionState,
}

impl StateImages for IniSet<'_> {
    fn state(&self) -> Option<RegionState> {
        Some(self.state)
    }

    fn name(&self) -> &str {
        match self.state {
            RegionState::Hover => "Hover",
            RegionState::Down => "Down",
            RegionState::Disabled => "Disabled",
            _ => "Normal",
        }
    }

    fn count(&self) -> usize {
        let colour = |k: &str| {
            k.get(self.prefix.len() + 1..)
                .is_some_and(|r| r.to_ascii_lowercase().starts_with("colorgroup"))
        };
        self.section
            .entries
            .iter()
            .filter(|e| is_prefixed(&e.key, self.prefix) && !colour(&e.key))
            .count()
    }

    fn piece(&self, ty: &str, background: bool) -> Result<Option<RgbaImage>, ImportError> {
        let Some(value) = self.section.get(&format!("{}.{ty}", self.prefix)) else {
            return Ok(None);
        };
        let (file, bytes) = match resolve(self.images.tree, value, self.images.aliases) {
            Err(ImportError::UndefinedBitmap { name }) => {
                self.images.absent.borrow_mut().insert(name);
                return Ok(None);
            }
            other => other?,
        };
        self.images.files.borrow_mut().insert(file.to_owned());
        let mut img = bitmap::decode(file, bytes)?;
        let group = if background {
            "ColorGroupBG"
        } else {
            "ColorGroup"
        };
        // An undefined group is the identity shift (d1 §3.1).
        let by = self
            .section
            .get(&format!("{}.{group}", self.prefix))
            .and_then(|c| self.images.colours.iter().find(|r| r.name == c))
            .map_or([0; 3], |r| r.value);
        bitmap::shift(&mut img, by);
        Ok(Some(img))
    }
}

/// Whether `key` of a control section that filled a slot was read: a
/// state's piece of a slot, or its colour group.
#[must_use]
pub fn is_read(key: &str) -> bool {
    STATES.iter().any(|(p, _)| {
        is_prefixed(key, p)
            && (is_read_piece(&key[p.len() + 1..])
                || surfaces::is_read_piece(&key[p.len() + 1..])
                || key[p.len() + 1..].eq_ignore_ascii_case("ColorGroup")
                || key[p.len() + 1..].eq_ignore_ascii_case("ColorGroupBG"))
    })
}

/// Whether `key` is `<prefix>.<something>` (ASCII-case-insensitive).
pub fn is_prefixed(key: &str, prefix: &str) -> bool {
    key.get(..prefix.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
        && key[prefix.len()..].starts_with('.')
}

/// The controls of `skin`, the state tally, and the sections read.
///
/// # Errors
/// A missing or refused image.
pub fn read(
    skin: &Ini,
    images: &Images<'_>,
) -> Result<(Vec<Control>, StateTally, BTreeSet<String>), ImportError> {
    let mut controls = Vec::new();
    let mut tally = StateTally::default();
    let used = read_file(skin, images, Window::Main, (&mut controls, &mut tally))?;
    Ok((controls, tally, used))
}

/// Fills `controls` from the sections of one skin file, `window`: the
/// main skin file's title buttons, scrollbar and surfaces, or a chat
/// window file's surfaces only (plan E7.7.7 S2). Returns the sections read.
///
/// # Errors
/// A missing or refused image.
pub fn read_file(
    ini: &Ini,
    images: &Images<'_>,
    window: Window,
    (controls, tally): (&mut Vec<Control>, &mut StateTally),
) -> Result<BTreeSet<String>, ImportError> {
    let mut used = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for section in &ini.sections {
        if !seen.insert(section.name.to_ascii_lowercase()) {
            continue;
        }
        let sets: Vec<IniSet<'_>> = STATES
            .iter()
            .filter(|(p, _)| section.entries.iter().any(|e| is_prefixed(&e.key, p)))
            .map(|&(prefix, state)| IniSet {
                images,
                section,
                prefix,
                state,
            })
            .collect();
        let filled = (window == Window::Main
            && fill(&section.name, section.get("Class"), &sets, controls, tally)?)
            || surfaces::fill((&section.name, window, true), &sets, controls, tally)?;
        if filled {
            used.insert(section.name.to_ascii_lowercase());
        }
    }
    Ok(used)
}
