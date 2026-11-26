//! Who draws each interactive node's disabled look (plan E7.7 CR1, owner
//! ruling: a disabled button is visibly disabled and shows the Host's
//! reason): a serve pass ([`crate::manifest::place`]), never written into
//! a package.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{RegionState, SkinError, SkinRule, WindowLayout};
use rekindle_types::dto::skin_tree::{
    DisabledBy, NodeArt, NodePiece, NodePieces, NodeRole, SkinNode, SkinTree,
};

use super::dim::{self, Made, Under};
use crate::package::Package;

/// The largest channel difference between a Disabled pixel and its Normal
/// pixel that is still the same colour: an eighth of the range, above the
/// re-encoding and anti-aliasing noise of an imported pack and above the
/// icon tolerance (24) the cover pass reads a ground by
/// ([`super::cover::ICON_TOLERANCE`]).
pub(super) const CHANNEL_DELTA: u8 = 32;

/// The fewest visibly changed pixels that make a Disabled set a look of
/// its own: the bar the real-app proof holds the shell's dimming to (a
/// visibly disabled control changes at least 20 px).
pub(super) const VISIBLE_PIXELS: u64 = 20;

/// What this serve already measured and made: the visibly changed pixel
/// count of each (Normal, Disabled) image pair, the first frame of each
/// image decoded for it by name (a made copy's frame under its made name,
/// [`dim`]), and the Disabled copies made ([`dim::copy`]). Names are content hashes, so a pair measures
/// the same in every window and node that shows it, and an image shared by
/// several pairs (one Normal under two Disabled looks) decodes once.
/// [`crate::manifest::place`] makes one per serve: it runs once per
/// manifest, every window inside it.
#[derive(Default)]
pub struct Measured {
    pairs: HashMap<(String, String), u64>,
    pub(super) frames: HashMap<String, RgbaImage>,
    pub(super) made: Made,
}

impl Measured {
    /// The Disabled copies this serve made, by name.
    #[must_use]
    pub fn made(self) -> Made {
        self.made
    }
}

/// Sets `disabled_by` on every interactive node of the window's tree and
/// its popups' trees, laid in the window at its served minimum (as
/// [`super::target::nodes`] lays them), serves a Disabled copy for each one
/// the serve draws ([`look`]), and clears it on every other node.
///
/// # Errors
/// A Disabled or Normal piece missing from `package` or not decoding, or a
/// copy not encoding.
pub fn nodes(
    layout: &mut WindowLayout,
    package: &Package<'_>,
    measured: &mut Measured,
) -> Result<(), SkinError> {
    let at = super::target::window(layout);
    if let Some(tree) = layout.tree.as_mut() {
        laid(tree, at, package, measured)?;
    }
    let min_height = layout.min_height;
    for popup in layout.popups.values_mut() {
        let at = super::target::window(&super::popup::frame(popup, min_height));
        laid(&mut popup.tree, at, package, measured)?;
    }
    Ok(())
}

fn laid(
    tree: &mut SkinTree,
    (w, h): (i32, i32),
    package: &Package<'_>,
    measured: &mut Measured,
) -> Result<(), SkinError> {
    let boxes: HashMap<String, [i32; 4]> = tree
        .boxes(w, h)
        .into_iter()
        .map(|(n, b)| (n.id.clone(), b))
        .collect();
    mark(&mut tree.root, None, &boxes, package, measured)
}

fn mark(
    nodes: &mut [SkinNode],
    under: Option<&Under>,
    boxes: &HashMap<String, [i32; 4]>,
    package: &Package<'_>,
    measured: &mut Measured,
) -> Result<(), SkinError> {
    for n in nodes {
        n.disabled_by = if n.interactive() {
            Some(look(n, under, boxes, package, measured)?)
        } else {
            None
        };
        let own = n
            .art
            .as_ref()
            .filter(|a| !a.normal.is_empty())
            .zip(boxes.get(&n.id))
            .map(|(a, at)| Under {
                set: a.normal.clone(),
                at: *at,
            });
        // The Host's mark covers a branded node's whole box: nothing of
        // the art above it is seen inside it.
        let inside = if super::brand::branded(n) {
            None
        } else {
            own.as_ref().or(under)
        };
        mark(&mut n.children, inside, boxes, package, measured)?;
    }
    Ok(())
}

/// Who draws the interactive node `n`'s disabled look: the pack, when its
/// Disabled art is visibly its own ([`pack_art`]); the serve, with a
/// dimmed copy of what is painted at its box set as its Disabled layer
/// ([`dim::copy`]), when that copy changes [`VISIBLE_PIXELS`]; else the
/// shell's opacity, which also dims the node's children and words: a node
/// that or whose parts carry words (a Host label or the pack's text, which a
/// copy of the art cannot carry), one whose parts draw art over it (a layer
/// of the node lies under them), a branded node (the Host's mark, never the
/// pack's art, [`super::brand`]), one sized by its text at render (no box
/// at serve, [`super::validate::text_sized`]), and one with nothing visibly
/// dimmable at its box (an art-less grip over no art).
fn look(
    n: &mut SkinNode,
    under: Option<&Under>,
    boxes: &HashMap<String, [i32; 4]>,
    package: &Package<'_>,
    measured: &mut Measured,
) -> Result<DisabledBy, SkinError> {
    if pack_art(n, package, measured)? {
        return Ok(DisabledBy::Pack);
    }
    if shell_only(n) {
        return Ok(DisabledBy::Shell);
    }
    let Some(&at) = boxes.get(&n.id) else {
        return Ok(DisabledBy::Shell);
    };
    let Some(set) = dim::copy(n, at, under, package, measured)? else {
        return Ok(DisabledBy::Shell);
    };
    n.art
        .get_or_insert_with(|| NodeArt {
            normal: NodePieces::new(),
            states: std::collections::BTreeMap::new(),
        })
        .states
        .insert(RegionState::Disabled, set);
    Ok(DisabledBy::Served)
}

/// What only the shell's opacity can dim (see [`look`]): words on the node
/// or its parts, parts with art, the Host's mark, a text-sized box.
pub(super) fn shell_only(n: &SkinNode) -> bool {
    let worded = |c: &SkinNode| c.label.is_some() || c.text.is_some();
    let parts = n.children.iter().filter(|c| c.role == NodeRole::Part);
    worded(n)
        || parts.clone().any(|c| worded(c) || c.art.is_some())
        || super::brand::branded(n)
        || super::validate::text_sized(n)
}

/// The pack ships a disabled look of its own for the node's art or one of
/// its parts' (the importer's `--disabled` state variant whose layers
/// differ visibly from the normal ones: at least [`VISIBLE_PIXELS`] pixels
/// change by more than [`CHANNEL_DELTA`] in a channel, as xSLIMFIRE's chat
/// button's greyed art does), and the Host draws no words over it (a Host
/// label, such as the Communities tab's, is the shell's to dim). A
/// Disabled set equal to Normal, or one that differs by a few pixels (Total
/// Chaos's `CommunitiesOpen`), would leave a refused button looking
/// pressable, so the shell dims it (owner rule: never hidden, visibly
/// disabled).
///
/// # Errors
/// As [`nodes`].
pub(super) fn pack_art(
    n: &SkinNode,
    package: &Package<'_>,
    measured: &mut Measured,
) -> Result<bool, SkinError> {
    if n.label.is_some() {
        return Ok(false);
    }
    let parts = n.children.iter().filter(|c| c.role == NodeRole::Part);
    for node in std::iter::once(n).chain(parts) {
        let Some(art) = node.art.as_ref() else {
            continue;
        };
        let Some(disabled) = art.states.get(&RegionState::Disabled) else {
            continue;
        };
        if changed_pixels(&art.normal, disabled, package, measured)? >= VISIBLE_PIXELS {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The visibly changed pixels from the `normal` piece set to `disabled`,
/// slot by slot; a slot only one set has counts all its pixels.
pub(super) fn changed_pixels(
    normal: &NodePieces,
    disabled: &NodePieces,
    package: &Package<'_>,
    measured: &mut Measured,
) -> Result<u64, SkinError> {
    let mut total = 0u64;
    for (slot, d) in disabled {
        total += match normal.get(slot) {
            None => area(d),
            Some(a) if a.image == d.image => 0,
            Some(a) => {
                let key = (a.image.clone(), d.image.clone());
                if let Some(&n) = measured.pairs.get(&key) {
                    n
                } else {
                    load(a, package, &mut measured.frames)?;
                    load(d, package, &mut measured.frames)?;
                    let n =
                        visible_difference(&measured.frames[&a.image], &measured.frames[&d.image]);
                    measured.pairs.insert(key, n);
                    n
                }
            }
        };
    }
    for (slot, a) in normal {
        if !disabled.contains_key(slot) {
            total += area(a);
        }
    }
    Ok(total)
}

fn area(p: &NodePiece) -> u64 {
    u64::from(p.width) * u64::from(p.height)
}

/// Decodes the first frame of `p`'s image into `frames`, once per serve.
pub(super) fn load(
    p: &NodePiece,
    package: &Package<'_>,
    frames: &mut HashMap<String, RgbaImage>,
) -> Result<(), SkinError> {
    if let Entry::Vacant(slot) = frames.entry(p.image.clone()) {
        let missing = |detail: &str| {
            SkinError::new(
                p.image.as_str(),
                SkinRule::MissingEntry {
                    field: format!("art.{detail}"),
                },
            )
        };
        let bytes = package.get(&p.image).ok_or_else(|| missing("image"))?;
        let first = crate::regions::frames(&p.image, bytes)?
            .into_iter()
            .next()
            .ok_or_else(|| missing("frame"))?;
        slot.insert(first);
    }
    Ok(())
}

/// The pixels of `a` and `b` that differ by more than [`CHANNEL_DELTA`] in
/// a channel (fully transparent pixels compare as one colour); images of
/// different sizes are different art, every pixel of the larger counted.
pub(super) fn visible_difference(a: &RgbaImage, b: &RgbaImage) -> u64 {
    if a.dimensions() != b.dimensions() {
        let (aw, ah) = a.dimensions();
        let (bw, bh) = b.dimensions();
        return (u64::from(aw) * u64::from(ah)).max(u64::from(bw) * u64::from(bh));
    }
    let count = a
        .pixels()
        .zip(b.pixels())
        .filter(|(p, q)| {
            super::cover::seen(p.0)
                .iter()
                .zip(super::cover::seen(q.0))
                .any(|(x, y)| x.abs_diff(y) > CHANNEL_DELTA)
        })
        .count();
    u64::try_from(count).unwrap_or(u64::MAX)
}
