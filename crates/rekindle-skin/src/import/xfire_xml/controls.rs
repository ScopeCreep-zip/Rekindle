//! The painted control slots of an Xfire skin, XML or INI era (plan E7.7.1
//! "Regions"; E7.7.3 item 1): the title buttons and the scrollbar, each
//! state image set a region state (Normal or INI `Graphics` → normal,
//! Hover → hover, Down or INI `BtnDown` → down, Disabled → disabled).
//!
//! * A title button is the first component (XML) or section (INI) bearing
//!   one of its names ([`button_slot`]: Symbiosis's `ButtonMinimize`, the
//!   fan trees' `Minimize`, `MinimizeBox`, `WndMin` …): the `Middle` image,
//!   `stretch` (Xfire's button into the app's titlebar button).
//! * The scrollbar is the first `XfireScrollbar` or `Class="Scrollbar"`
//!   ([`is_scrollbar`]): the three gripper pieces of each axis stacked into
//!   a thumb and the three gutter pieces into a track, `nine-slice` with
//!   the end pieces as insets; the four arrows and the corner, `stretch`.
//!   The gripper overlays (the grip lines Xfire centres on the thumb) have
//!   no place in a nine-slice and are dropped. The arrows and grippers
//!   take the set's foreground shift (`Color`, INI `ColorGroup`), the
//!   gutters and corner its background one (`ColorBG`, `ColorGroupBG`).
//! * A state whose image differs in size from the normal one is dropped
//!   (the validator refuses it), and named in the `NOTICE`.
//! * The buddy-list and chat windows' bars and buttons fill their surface
//!   slots ([`super::surfaces`], both eras; plan E7.7.7).
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

/// The file name a composed control image's errors carry (Symbiosis's
/// controls file).
pub const SOURCE: &str = "Components/Common.xml";

/// The title buttons: the names a component (XML) or section (INI) has
/// across the Xfire trees, → slot. Symbiosis `ButtonMinimize`; Onyx,
/// Space Cowboy, Total Chaos `Minimize`; Alloy, Simple X, xSLIMFIRE
/// `MinimizeBox`; XGlow `MinimizeButton`; StealthX `WndMin`; and the chat
/// window's own `ChatMinimize` (Total Chaos). The first component or
/// section in document order fills a slot.
const BUTTONS: [(&[&str], &str); 3] = [
    (
        &[
            "ButtonMinimize",
            "Minimize",
            "MinimizeBox",
            "MinimizeButton",
            "WndMin",
            "ChatMinimize",
        ],
        "titlebar.btn.minimize",
    ),
    (
        &[
            "ButtonMaximize",
            "Maximize",
            "MaximizeBox",
            "MaximizeButton",
            "WndMax",
            "ChatMaximize",
        ],
        "titlebar.btn.maximize",
    ),
    (
        &[
            "ButtonClose",
            "Close",
            "CloseBox",
            "CloseButton",
            "WndClose",
            "ChatClose",
        ],
        "titlebar.btn.close",
    ),
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

/// Whether a piece type is one a slot reads (a button's `Middle`, a
/// scrollbar part's pieces), ASCII-case-insensitively.
#[must_use]
pub fn is_read_piece(ty: &str) -> bool {
    ty.eq_ignore_ascii_case("Middle")
        || PARTS
            .iter()
            .flat_map(|p| p.pieces)
            .any(|piece| piece.eq_ignore_ascii_case(ty))
}

/// The state an `<Images State=…>` set is (ASCII-case-insensitive, as
/// the Windows client matched names).
fn state(name: Option<&str>) -> Option<RegionState> {
    let name = name.unwrap_or("Normal");
    [
        ("Normal", RegionState::Normal),
        ("Hover", RegionState::Hover),
        ("Down", RegionState::Down),
        ("Disabled", RegionState::Disabled),
    ]
    .into_iter()
    .find(|(k, _)| k.eq_ignore_ascii_case(name))
    .map(|(_, s)| s)
}

/// The title button slot a component or INI section named `name` fills
/// (ASCII-case-insensitive).
#[must_use]
pub fn button_slot(name: &str) -> Option<&'static str> {
    BUTTONS
        .iter()
        .find(|(names, _)| names.iter().any(|n| n.eq_ignore_ascii_case(name)))
        .map(|(_, region)| *region)
}

/// Whether a component or INI section is a scrollbar: Symbiosis's
/// [`SCROLLBAR`], or any `Class="Scrollbar"`.
#[must_use]
pub fn is_scrollbar(name: &str, class: Option<&str>) -> bool {
    name == SCROLLBAR || class.is_some_and(|c| c.eq_ignore_ascii_case("Scrollbar"))
}

/// One state's image set of a control, from either era: an XML
/// `<Images State=…>` or an INI section's `<Prefix>.*` keys.
pub trait StateImages {
    /// The region state the set is; `None` for a state no slot has.
    fn state(&self) -> Option<RegionState>;
    /// The set's name, for the tally (`Hover`, `BtnDown`, …).
    fn name(&self) -> &str;
    /// How many images the set names.
    fn count(&self) -> usize;
    /// The piece typed `ty` (ASCII-case-insensitive), decoded and shifted by
    /// the set's foreground colour, or its background colour for
    /// `background`; `None` when the set does not name it.
    ///
    /// # Errors
    /// A missing or refused bitmap.
    fn piece(&self, ty: &str, background: bool) -> Result<Option<RgbaImage>, ImportError>;
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
    /// The component (XML) or section (INI) each painted slot was read from.
    pub sources: BTreeMap<&'static str, String>,
}

/// The decoder of one skin's images.
struct Decoder<'a> {
    tree: &'a Tree,
    bitmaps: BTreeMap<&'a str, &'a str>,
    colours: &'a [ColourRow],
}

/// An XML `<Images State=…>` set.
struct XmlSet<'a> {
    d: &'a Decoder<'a>,
    set: &'a Node,
}

impl StateImages for XmlSet<'_> {
    fn state(&self) -> Option<RegionState> {
        state(self.set.attr_ci("State"))
    }

    fn name(&self) -> &str {
        self.set.attr_ci("State").unwrap_or("Normal")
    }

    fn count(&self) -> usize {
        self.set.children_named("Image").count()
    }

    fn piece(&self, ty: &str, background: bool) -> Result<Option<RgbaImage>, ImportError> {
        let Some(img) = self.set.children_named("Image").find(|i| {
            i.attr_ci("Type")
                .is_some_and(|t| t.eq_ignore_ascii_case(ty))
        }) else {
            return Ok(None);
        };
        let shift = self
            .set
            .attr_ci(if background { "ColorBG" } else { "Color" });
        super::piece(
            self.d.tree,
            &self.d.bitmaps,
            self.d.colours,
            &img.text,
            shift,
        )
        .map(Some)
    }
}

/// A theme bitmap read by name as one state's `Middle` image (Symbiosis's
/// dialog buttons, [`super::surfaces::NAMED_BUTTONS`]).
struct NamedSet<'a> {
    d: &'a Decoder<'a>,
    bitmap: String,
    state: RegionState,
}

impl StateImages for NamedSet<'_> {
    fn state(&self) -> Option<RegionState> {
        Some(self.state)
    }

    fn name(&self) -> &str {
        set_name(self.state)
    }

    fn count(&self) -> usize {
        1
    }

    fn piece(&self, ty: &str, _background: bool) -> Result<Option<RgbaImage>, ImportError> {
        if !ty.eq_ignore_ascii_case("Middle") {
            return Ok(None);
        }
        super::piece(
            self.d.tree,
            &self.d.bitmaps,
            self.d.colours,
            &self.bitmap,
            None,
        )
        .map(Some)
    }
}

/// The named dialog buttons the theme has bitmaps for.
fn named_buttons(
    d: &Decoder<'_>,
    controls: &mut Vec<Control>,
    tally: &mut StateTally,
) -> Result<(), ImportError> {
    for (name, region) in super::surfaces::NAMED_BUTTONS {
        let sets: Vec<NamedSet<'_>> = super::surfaces::NAMED_STATES
            .iter()
            .map(|(suffix, state)| (format!("{name}_{suffix}"), *state))
            .filter(|(bitmap, _)| d.bitmaps.keys().any(|k| k.eq_ignore_ascii_case(bitmap)))
            .map(|(bitmap, state)| NamedSet { d, bitmap, state })
            .collect();
        super::surfaces::fill_named((name, region), &sets, controls, tally)?;
    }
    Ok(())
}

/// `pieces` of one axis as one image and its nine-slice, through
/// [`bitmap::compose`] with empty cells off the axis.
pub(super) fn three(
    pieces: [RgbaImage; 3],
    vertical: bool,
) -> Result<(RgbaImage, RegionFit), ImportError> {
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

/// Counts a control's non-normal sets into `tally`.
pub(super) fn count_sets<S: StateImages>(tally: &mut StateTally, sets: &[S]) {
    for set in sets
        .iter()
        .filter(|s| s.state() != Some(RegionState::Normal))
    {
        tally.sets.entry(static_state(set.name())).or_default().1 += 1;
        tally.images.1 += set.count();
    }
}

/// Builds the control(s) of `sets` when `name`/`class` fill a slot not yet
/// in `controls`; `None` when they fill none (the caller tallies the drop).
///
/// # Errors
/// A missing or refused bitmap.
pub fn fill<S: StateImages>(
    name: &str,
    class: Option<&str>,
    sets: &[S],
    controls: &mut Vec<Control>,
    tally: &mut StateTally,
) -> Result<bool, ImportError> {
    let taken = |region: &str| controls.iter().any(|c| c.region == region);
    let made = match button_slot(name) {
        Some(region) if !taken(region) => button(region, sets)?,
        _ if is_scrollbar(name, class)
            && !taken("scrollbar.thumb")
            && !taken("scrollbar.track") =>
        {
            scrollbar(sets)?
        }
        _ => return Ok(false),
    };
    count_sets(tally, sets);
    let mut kept_states = std::collections::BTreeSet::new();
    for (control, sizes_dropped, kept_images) in made {
        for (st, why) in sizes_dropped {
            tally
                .dropped
                .push((format!("{name} ({})", control.region), st, why));
        }
        kept_states.extend(control.images.keys().filter(|s| **s != RegionState::Normal));
        tally.images.0 += kept_images;
        tally.sources.insert(control.region, name.to_owned());
        controls.push(control);
    }
    for st in kept_states {
        tally.sets.entry(set_name(st)).or_default().0 += 1;
    }
    Ok(true)
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
        let sets: Vec<XmlSet<'_>> = c
            .children_named("Images")
            .map(|set| XmlSet { d: &d, set })
            .collect();
        if fill(name, c.attr_ci("Class"), &sets, &mut controls, &mut tally)?
            || super::surfaces::fill(
                (name, super::surfaces::Window::Main, false),
                &sets,
                &mut controls,
                &mut tally,
            )?
        {
            continue;
        }
        let why = if c.attr_ci("Class") == Some("Tile") && name.contains("Gripper") {
            "no element: the OS resizes the window"
        } else if button_slot(name).is_some() || is_scrollbar(name, c.attr_ci("Class")) {
            "its slot is filled by an earlier component"
        } else {
            "no region slot: its bitmap carries a label, icon or Xfire-only control"
        };
        for set in sets
            .iter()
            .filter(|s| s.state() != Some(RegionState::Normal))
        {
            tally.sets.entry(static_state(set.name())).or_default().1 += 1;
            tally.images.1 += set.count();
            tally
                .dropped
                .push((name.to_owned(), static_state(set.name()), why));
        }
    }
    named_buttons(&d, &mut controls, &mut tally)?;
    Ok((controls, tally))
}

/// Dropped state images: (state, why).
type Dropped = Vec<(&'static str, &'static str)>;
/// Each control, its dropped states and how many source images it kept.
type Made = Vec<(Control, Dropped, usize)>;

pub(super) fn set_name(s: RegionState) -> &'static str {
    match s {
        RegionState::Hover => "Hover",
        RegionState::Down => "Down",
        // A titlebar's Disabled set, drawn while the window is inactive.
        RegionState::Disabled | RegionState::WindowInactive => "Disabled",
        _ => "Normal",
    }
}

fn static_state(s: &str) -> &'static str {
    ["Hover", "Down", "Disabled", "Normal"]
        .into_iter()
        .find(|k| k.eq_ignore_ascii_case(s))
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

/// The app's title button box, CSS px (`00-rules.css` `.xfire-titlebar-btn`,
/// 24 × 24 for SC 2.5.8).
const TITLE_BUTTON: u32 = 24;

/// A title button's art centred on a transparent square at least the
/// button's box, so `stretch` draws it at 1:1 (or scaled evenly when it is
/// larger) instead of pulling xSLIMFIRE's 9 × 5 LED into a 24 × 24 smear.
fn centred(img: RgbaImage) -> RgbaImage {
    let side = TITLE_BUTTON.max(img.width()).max(img.height());
    if img.width() == side && img.height() == side {
        return img;
    }
    let mut out = RgbaImage::new(side, side);
    let (dx, dy) = ((side - img.width()) / 2, (side - img.height()) / 2);
    for (x, y, p) in img.enumerate_pixels() {
        out.put_pixel(x + dx, y + dy, *p);
    }
    out
}

fn button<S: StateImages>(region: &'static str, sets: &[S]) -> Result<Made, ImportError> {
    let mut control = Control {
        region,
        fit: RegionFit::Stretch,
        images: BTreeMap::new(),
    };
    let mut dropped = Vec::new();
    let mut kept = 0;
    for set in sets {
        let Some(st) = set.state() else {
            continue;
        };
        let Some(img) = set.piece("Middle", false)? else {
            continue;
        };
        if keep(&mut control, st, img, &mut dropped) {
            kept += 1;
        }
    }
    if !control.images.contains_key(&RegionState::Normal) {
        return Ok(Vec::new());
    }
    // After the size check, so a state of another size is still dropped.
    for img in control.images.values_mut() {
        *img = centred(std::mem::replace(img, RgbaImage::new(0, 0)));
    }
    Ok(vec![(control, dropped, kept)])
}

fn scrollbar<S: StateImages>(sets: &[S]) -> Result<Made, ImportError> {
    let mut out = Vec::new();
    for p in &PARTS {
        let mut control = Control {
            region: p.region,
            fit: RegionFit::Stretch,
            images: BTreeMap::new(),
        };
        let mut dropped = Vec::new();
        let mut kept = 0;
        for set in sets {
            let Some(st) = set.state() else {
                continue;
            };
            let mut pieces = Vec::new();
            for ty in p.pieces {
                if let Some(img) = set.piece(ty, p.background)? {
                    pieces.push(img);
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
