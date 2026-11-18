//! The painted control slots of an Xfire XML-era skin (plan E7.7.1
//! "Regions"; E7.7.3 item 1): `Components/Common.xml`'s title buttons and
//! its scrollbar, each `<Images State=…>` set a region state (Normal →
//! normal, Hover → hover, Down → down, Disabled → disabled).
//!
//! * `ButtonMinimize`, `ButtonMaximize`, `ButtonClose`: the `Middle` image,
//!   `stretch` (Xfire's 18 × 16 button into the app's titlebar button).
//! * `XfireScrollbar`: the three gripper pieces of each axis stacked into
//!   a thumb and the three gutter pieces into a track, `nine-slice` with
//!   the end pieces as insets; the four arrows and the corner, `stretch`.
//!   The gripper overlays (the grip lines Xfire centres on the thumb) have
//!   no place in a nine-slice and are dropped. The arrows and grippers
//!   take the set's `Color` shift, the gutters and corner its `ColorBG`.
//! * A state whose image differs in size from the normal one is dropped
//!   (the validator refuses it), and named in the `NOTICE`.
//! * Every other state set stays dropped: Xfire's other push buttons,
//!   menus, tabs and launchers bake their label or icon into the bitmap,
//!   and a region never replaces a label (plan E7.7.1); the resize
//!   grippers have no element (the OS resizes the window).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{RegionFit, RegionState};

use super::super::tree::Node;
use super::super::{Control, ImportError, Tree};
use super::{bitmap, descendants, ColourRow};

/// Where the controls are read.
pub const SOURCE: &str = "Components/Common.xml";

/// The title buttons: component → slot.
const BUTTONS: [(&str, &str); 3] = [
    ("ButtonMinimize", "titlebar.btn.minimize"),
    ("ButtonMaximize", "titlebar.btn.maximize"),
    ("ButtonClose", "titlebar.btn.close"),
];
const SCROLLBAR: &str = "XfireScrollbar";

/// A scrollbar slot from its pieces: one piece is `stretch`; three are
/// stacked (vertical) or laid side by side and `nine-slice`d.
struct Part {
    region: &'static str,
    pieces: &'static [&'static str],
    vertical: bool,
    background: bool,
}

const PARTS: [Part; 9] = [
    part("scrollbar.button-up", &["ScrollUp"], false),
    part("scrollbar.button-down", &["ScrollDown"], false),
    part("scrollbar.button-left", &["ScrollLeft"], false),
    part("scrollbar.button-right", &["ScrollRight"], false),
    Part {
        region: "scrollbar.thumb",
        pieces: &[
            "ScrollVertGripperTop",
            "ScrollVertGripperMiddle",
            "ScrollVertGripperBottom",
        ],
        vertical: true,
        background: false,
    },
    part(
        "scrollbar.thumb-horizontal",
        &[
            "ScrollHorzGripperLeft",
            "ScrollHorzGripperMiddle",
            "ScrollHorzGripperRight",
        ],
        false,
    ),
    Part {
        region: "scrollbar.track",
        pieces: &[
            "ScrollVertGutterTop",
            "ScrollVertGutterMiddle",
            "ScrollVertGutterBottom",
        ],
        vertical: true,
        background: true,
    },
    part(
        "scrollbar.track-horizontal",
        &[
            "ScrollHorzGutterLeft",
            "ScrollHorzGutterMiddle",
            "ScrollHorzGutterRight",
        ],
        true,
    ),
    part("scrollbar.corner", &["ScrollCorner"], true),
];

const fn part(region: &'static str, pieces: &'static [&'static str], background: bool) -> Part {
    Part {
        region,
        pieces,
        vertical: false,
        background,
    }
}

/// The state an `<Images State=…>` set is.
fn state(name: Option<&str>) -> Option<RegionState> {
    match name.unwrap_or("Normal") {
        "Normal" => Some(RegionState::Normal),
        "Hover" => Some(RegionState::Hover),
        "Down" => Some(RegionState::Down),
        "Disabled" => Some(RegionState::Disabled),
        _ => None,
    }
}

/// What the controls kept and dropped, for the `NOTICE`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateTally {
    /// State image sets (Hover, Down, Disabled) per state: (kept, of).
    pub sets: BTreeMap<&'static str, (usize, usize)>,
    /// Images of those sets: (kept, of).
    pub images: (usize, usize),
    /// Each dropped state image set: component and why.
    pub dropped: Vec<(String, &'static str, &'static str)>,
}

/// The decoder of one skin's images.
struct Decoder<'a> {
    tree: &'a Tree,
    bitmaps: BTreeMap<&'a str, &'a str>,
    colours: &'a [ColourRow],
}

impl Decoder<'_> {
    /// Bitmap `name` decoded and shifted by the colour named `shift`.
    fn image(&self, name: &str, shift: Option<&str>) -> Result<RgbaImage, ImportError> {
        super::piece(self.tree, &self.bitmaps, self.colours, name, shift)
    }
}

/// `pieces` of one axis as one image and its nine-slice, through
/// [`bitmap::compose`] with empty cells off the axis.
fn three(pieces: [RgbaImage; 3], vertical: bool) -> Result<(RgbaImage, RegionFit), ImportError> {
    let e = RgbaImage::new(0, 0);
    let [a, b, c] = pieces;
    let nine = if vertical {
        [
            e.clone(),
            a,
            e.clone(),
            e.clone(),
            b,
            e.clone(),
            e.clone(),
            c,
            e,
        ]
    } else {
        [
            e.clone(),
            e.clone(),
            e.clone(),
            a,
            b,
            c,
            e.clone(),
            e.clone(),
            e,
        ]
    };
    let (img, insets) = bitmap::compose(SOURCE, &nine)?;
    Ok((img, RegionFit::NineSlice(insets)))
}

/// Reads the controls of `components` (every included component file).
///
/// # Errors
/// A missing or refused bitmap.
pub fn read(
    tree: &Tree,
    theme: &Node,
    colours: &[ColourRow],
    components: &[Node],
) -> Result<(Vec<Control>, StateTally), ImportError> {
    let d = Decoder {
        tree,
        bitmaps: super::bitmaps(theme),
        colours,
    };
    let mut tally = StateTally::default();
    let mut controls = Vec::new();
    for c in components.iter().flat_map(|n| descendants(n, "Component")) {
        let name = c.attr_ci("Name").unwrap_or_default();
        let sets: Vec<&Node> = c
            .children_named("Images")
            .filter(|i| state(i.attr_ci("State")) != Some(RegionState::Normal))
            .collect();
        for set in &sets {
            let s = set.attr_ci("State").unwrap_or_default();
            let entry = tally.sets.entry(static_state(s)).or_default();
            entry.1 += 1;
            tally.images.1 += set.children_named("Image").count();
        }
        let made = if let Some((_, region)) = BUTTONS.iter().find(|(n, _)| *n == name) {
            button(&d, c, region)?
        } else if name == SCROLLBAR {
            scrollbar(&d, c)?
        } else {
            for set in sets {
                let why = if c.attr_ci("Class") == Some("Tile") && name.contains("Gripper") {
                    "no element: the OS resizes the window"
                } else {
                    "no region slot: its bitmap carries a label, icon or Xfire-only control"
                };
                tally.dropped.push((
                    name.to_owned(),
                    static_state(set.attr_ci("State").unwrap_or_default()),
                    why,
                ));
            }
            continue;
        };
        let mut kept_states = std::collections::BTreeSet::new();
        for (control, sizes_dropped, kept_images) in made {
            for (st, why) in sizes_dropped {
                tally
                    .dropped
                    .push((format!("{name} ({})", control.region), st, why));
            }
            kept_states.extend(control.images.keys().filter(|s| **s != RegionState::Normal));
            tally.images.0 += kept_images;
            controls.push(control);
        }
        for st in kept_states {
            tally.sets.entry(set_name(st)).or_default().0 += 1;
        }
    }
    Ok((controls, tally))
}

/// Dropped state images: (state, why).
type Dropped = Vec<(&'static str, &'static str)>;
/// Each control, its dropped states and how many source images it kept.
type Made = Vec<(Control, Dropped, usize)>;

fn set_name(s: RegionState) -> &'static str {
    match s {
        RegionState::Hover => "Hover",
        RegionState::Down => "Down",
        RegionState::Disabled => "Disabled",
        _ => "Normal",
    }
}

fn static_state(s: &str) -> &'static str {
    ["Hover", "Down", "Disabled", "Normal"]
        .into_iter()
        .find(|k| *k == s)
        .unwrap_or("other")
}

/// Keeps `img` for `st` when it is the normal image's size.
fn keep(control: &mut Control, st: RegionState, img: RgbaImage, dropped: &mut Dropped) -> bool {
    let normal = control
        .images
        .get(&RegionState::Normal)
        .map(RgbaImage::dimensions);
    if st == RegionState::Normal || normal == Some(img.dimensions()) {
        control.images.insert(st, img);
        st != RegionState::Normal
    } else {
        dropped.push((set_name(st), "its size differs from the normal image's"));
        false
    }
}

fn button(d: &Decoder<'_>, c: &Node, region: &'static str) -> Result<Made, ImportError> {
    let mut control = Control {
        region,
        fit: RegionFit::Stretch,
        images: BTreeMap::new(),
    };
    let mut dropped = Vec::new();
    let mut kept = 0;
    for set in c.children_named("Images") {
        let Some(st) = state(set.attr_ci("State")) else {
            continue;
        };
        let Some(img) = set
            .children_named("Image")
            .find(|i| i.attr_ci("Type") == Some("Middle"))
        else {
            continue;
        };
        if keep(
            &mut control,
            st,
            d.image(&img.text, set.attr_ci("Color"))?,
            &mut dropped,
        ) {
            kept += 1;
        }
    }
    Ok(vec![(control, dropped, kept)])
}

fn scrollbar(d: &Decoder<'_>, c: &Node) -> Result<Made, ImportError> {
    let mut out = Vec::new();
    for p in &PARTS {
        let mut control = Control {
            region: p.region,
            fit: RegionFit::Stretch,
            images: BTreeMap::new(),
        };
        let mut dropped = Vec::new();
        let mut kept = 0;
        for set in c.children_named("Images") {
            let Some(st) = state(set.attr_ci("State")) else {
                continue;
            };
            let shift = set.attr(if p.background { "ColorBG" } else { "Color" });
            let mut pieces = Vec::new();
            for ty in p.pieces {
                if let Some(img) = set
                    .children_named("Image")
                    .find(|i| i.attr_ci("Type") == Some(ty))
                {
                    pieces.push(d.image(&img.text, shift)?);
                }
            }
            let img = match <[RgbaImage; 3]>::try_from(pieces) {
                Ok(three_pieces) => {
                    let (img, fit) = three(three_pieces, p.vertical)?;
                    control.fit = fit;
                    img
                }
                Err(mut one) if one.len() == 1 && p.pieces.len() == 1 => one.remove(0),
                Err(_) => continue,
            };
            if keep(&mut control, st, img, &mut dropped) {
                kept += p.pieces.len();
            }
        }
        if control.images.contains_key(&RegionState::Normal) {
            out.push((control, dropped, kept));
        }
    }
    Ok(out)
}
