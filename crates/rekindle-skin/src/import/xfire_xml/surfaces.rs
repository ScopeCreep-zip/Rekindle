//! The buddy-list window's surfaces of an XML-era Xfire skin (plan E7.7.7
//! S1): the component that draws each part of Xfire's main window, onto
//! the region slot of the Rekindle surface that does the same job.
//!
//! * A bar (`Tile` with `TopLeft`/`Top`/`TopRight`, or the list header's
//!   `HeaderLeft`/`HeaderMiddle`/`HeaderRight`) is its three pieces side by
//!   side, `nine-slice` with the end pieces as insets: Xfire tiles the
//!   middle and keeps the ends, as `border-image` does.
//! * A push button's `Middle` image per state. Xfire draws it at its own
//!   size (`MinWidth` = `MaxWidth`); our tab and action buttons are sized
//!   by their labels, so it is a `nine-slice` with [`BUTTON_INSET`] px
//!   edges. Its label or icon is baked into the centre, which the slot's
//!   backing covers (plan E7.7.1: a region never replaces a label), so
//!   only the bevel shows.
//! * Xfire's tab buttons are `PushType="Sticky"` in one `GroupId`
//!   (Symbiosis `Components/MainWindow.xml:910`): the chosen tab keeps its
//!   Down image, so Down also fills `selected`.
//! * The first component bearing a name fills its slot (document order,
//!   as [`super::controls`] does).
//!
//! Not mapped, each with its reason in the inventory (plan E7.7.7 S1):
//! the tab strip (`TabButtonsContainer`: our tabs fill the bar, so a strip
//! behind them never shows), the Xfire logo launcher (a trademark that
//! opens xfire.com; we have no such control), the ad view, and the other
//! tabs' and tab bars' buttons (one image per slot: the first tab's and the
//! friends bar's add button carry the look).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, RegionFit, RegionState};

use super::super::{Control, ImportError};
use super::controls::{count_sets, set_name as state_name, three, StateImages, StateTally};

/// A button image's edge kept at 1:1, px: Symbiosis's tab and friends
/// buttons have a 3 px bevel inside a 1 px outline.
pub const BUTTON_INSET: u16 = 4;

/// How a surface's images are read.
#[derive(Debug, Clone, Copy)]
enum Pieces {
    /// Three pieces side by side.
    Row([&'static str; 3]),
    /// One push button `Middle` image.
    Button { selected_from_down: bool },
}

const TOP: Pieces = Pieces::Row(["TopLeft", "Top", "TopRight"]);

/// Component name → slot, in the order the inventory lists them.
const SURFACES: [(&str, &str, Pieces); 8] = [
    ("StatusBarContainer", "statusbar", TOP),
    (
        "ButtonStatusChanger",
        "statusbar.arrow",
        Pieces::Button {
            selected_from_down: false,
        },
    ),
    ("MenuBackground", "buddylist.menubar", TOP),
    ("InfoBar", "buddylist.infobar", TOP),
    (
        "ButtonFriendsTab",
        "buddylist.tab",
        Pieces::Button {
            selected_from_down: true,
        },
    ),
    (
        "XfireView",
        "buddylist.group-header",
        Pieces::Row(["HeaderLeft", "HeaderMiddle", "HeaderRight"]),
    ),
    ("FriendsButtonBar", "buddylist.actionbar", TOP),
    (
        "ButtonAddFriend",
        "buddylist.action-button",
        Pieces::Button {
            selected_from_down: false,
        },
    ),
];

/// The fit of a button image: a nine-slice of [`BUTTON_INSET`] edges, or
/// `stretch` for the status arrow, which has no label to size it.
fn button_fit(region: &str) -> RegionFit {
    if region == "statusbar.arrow" {
        return RegionFit::Stretch;
    }
    RegionFit::NineSlice(Insets {
        top: BUTTON_INSET,
        right: BUTTON_INSET,
        bottom: BUTTON_INSET,
        left: BUTTON_INSET,
    })
}

/// One state's image of `pieces`, `None` when the set lacks a piece.
fn image<S: StateImages>(
    set: &S,
    pieces: Pieces,
) -> Result<Option<(RgbaImage, Option<RegionFit>)>, ImportError> {
    match pieces {
        Pieces::Button { .. } => Ok(set.piece("Middle", false)?.map(|img| (img, None))),
        Pieces::Row(types) => {
            let mut row = Vec::with_capacity(3);
            for ty in types {
                match set.piece(ty, false)? {
                    Some(img) => row.push(img),
                    None => return Ok(None),
                }
            }
            let Ok(row) = <[RgbaImage; 3]>::try_from(row) else {
                return Ok(None);
            };
            let (img, fit) = three(row, false)?;
            Ok(Some((img, Some(fit))))
        }
    }
}

/// Fills the surface slot of the component `name` from its `sets` when it
/// names one not yet in `controls`; `false` when it names none.
///
/// # Errors
/// A missing or refused bitmap.
pub fn fill<S: StateImages>(
    name: &str,
    sets: &[S],
    controls: &mut Vec<Control>,
    tally: &mut StateTally,
) -> Result<bool, ImportError> {
    let Some((_, region, pieces)) = SURFACES
        .iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
        .copied()
    else {
        return Ok(false);
    };
    if controls.iter().any(|c| c.region == region) {
        return Ok(false);
    }
    let mut images: BTreeMap<RegionState, RgbaImage> = BTreeMap::new();
    let mut fit = button_fit(region);
    for set in sets {
        let Some(st) = set.state() else { continue };
        let Some((img, row_fit)) = image(set, pieces)? else {
            continue;
        };
        let normal = images.get(&RegionState::Normal).map(RgbaImage::dimensions);
        if st != RegionState::Normal && normal != Some(img.dimensions()) {
            tally.dropped.push((
                format!("{name} ({region})"),
                state_name(st),
                "its size differs from the normal image's",
            ));
            continue;
        }
        if let Some(f) = row_fit {
            fit = f;
        }
        images.insert(st, img);
    }
    if !images.contains_key(&RegionState::Normal) {
        return Ok(false);
    }
    count_sets(tally, sets);
    for st in images.keys().filter(|s| **s != RegionState::Normal) {
        tally.sets.entry(state_name(*st)).or_default().0 += 1;
    }
    let per_image = match pieces {
        Pieces::Row(types) => types.len(),
        Pieces::Button { .. } => 1,
    };
    tally.images.0 += (images.len() - 1) * per_image;
    if let Pieces::Button {
        selected_from_down: true,
    } = pieces
    {
        if let Some(down) = images.get(&RegionState::Down).cloned() {
            images.insert(RegionState::Selected, down);
        }
    }
    tally.sources.insert(region, name.to_owned());
    controls.push(Control {
        region,
        fit,
        images,
    });
    Ok(true)
}
