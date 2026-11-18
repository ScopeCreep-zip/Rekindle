//! The INI era's window frame (plan E7.7.3 item 2): the skin file's frame
//! sections in one of [`SHAPES`], the first whose normal state is
//! complete, one image per complete state ([`super::STATES`]), each piece
//! shifted by its section's `<State>.ColorGroup` and keyed.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, RegionState};

use super::super::xfire_xml::{bitmap, ColourRow};
use super::super::{ImportError, Tree};
use super::ini::Ini;
use super::{resolve, STATES};

/// A frame shape: its pieces in [`bitmap::NINE`] order, (section, key);
/// `None` is the transparent middle.
pub type Shape = [Option<(&'static str, &'static str)>; 9];

/// DoiDD's frame: four sections, each side's `middle` the side piece.
pub const FRAME: Shape = [
    Some(("Top", "topleft")),
    Some(("Top", "top")),
    Some(("Top", "topright")),
    Some(("Left", "middle")),
    None,
    Some(("Right", "middle")),
    Some(("Bottom", "bottomleft")),
    Some(("Bottom", "bottom")),
    Some(("Bottom", "bottomright")),
];

/// Combat's frame (Xfire's own INI skin): one `[OuterFrame]` section naming
/// all eight pieces, the sides as `left`/`right`.
pub const OUTER_FRAME: Shape = [
    Some(("OuterFrame", "topleft")),
    Some(("OuterFrame", "top")),
    Some(("OuterFrame", "topright")),
    Some(("OuterFrame", "left")),
    None,
    Some(("OuterFrame", "right")),
    Some(("OuterFrame", "bottomleft")),
    Some(("OuterFrame", "bottom")),
    Some(("OuterFrame", "bottomright")),
];

/// Alloy's and Simple X's frame: `[Top]` and `[Bottom]` as DoiDD's, the
/// sides as `[Middle]`'s `left`/`right` (its `middle` is the content
/// background behind the window's views, not a frame piece).
pub const MIDDLE_FRAME: Shape = [
    Some(("Top", "topleft")),
    Some(("Top", "top")),
    Some(("Top", "topright")),
    Some(("Middle", "left")),
    None,
    Some(("Middle", "right")),
    Some(("Bottom", "bottomleft")),
    Some(("Bottom", "bottom")),
    Some(("Bottom", "bottomright")),
];

/// The shapes tried, in order: the first whose normal state is complete.
pub const SHAPES: [&Shape; 3] = [&FRAME, &MIDDLE_FRAME, &OUTER_FRAME];

/// The composed window frame: its insets and one image per complete state.
#[derive(Debug)]
pub struct Frame {
    /// The shape its pieces were read from.
    pub shape: &'static Shape,
    pub insets: Insets,
    pub images: BTreeMap<RegionState, RgbaImage>,
    /// The image files decoded for it.
    pub files: BTreeSet<String>,
    /// A state that was not emitted, and why.
    pub skipped: Vec<(RegionState, String)>,
}

/// One state's pieces (decoded only when all eight are named), shifted and
/// keyed, each decoded file added to `files`; or the keys it lacks.
fn state_pieces(
    tree: &Tree,
    skin: &Ini,
    shape: &Shape,
    prefix: &str,
    colours: &[ColourRow],
    aliases: &BTreeMap<String, String>,
    files: &mut BTreeSet<String>,
) -> Result<Result<[RgbaImage; 9], Vec<String>>, ImportError> {
    let mut missing = Vec::new();
    let mut named = Vec::with_capacity(8);
    for (sec, key) in shape.iter().flatten() {
        let s = skin.section(sec);
        match s.and_then(|s| s.get(&format!("{prefix}.{key}"))) {
            Some(value) => named.push((s, value)),
            None => missing.push(format!("[{sec}] {prefix}.{key}")),
        }
    }
    if !missing.is_empty() {
        return Ok(Err(missing));
    }
    let mut pieces: Vec<RgbaImage> = Vec::with_capacity(9);
    for (s, value) in named {
        let (file, bytes) = resolve(tree, value, aliases)?;
        files.insert(file.to_owned());
        let mut img = bitmap::decode(file, bytes)?;
        // An undefined group is the identity shift (d1 §3.1).
        let by = s
            .and_then(|s| s.get(&format!("{prefix}.ColorGroup")))
            .and_then(|c| colours.iter().find(|r| r.name == c))
            .map_or([0; 3], |r| r.value);
        bitmap::shift(&mut img, by);
        pieces.push(img);
    }
    // The transparent middle, between the left and right pieces.
    pieces.insert(4, RgbaImage::new(1, 1));
    Ok(Ok(pieces.try_into().map_err(|_| {
        ImportError::MissingFile {
            file: "frame pieces".to_owned(),
        }
    })?))
}

pub(super) fn read_frame(
    tree: &Tree,
    skin: &Ini,
    colours: &[ColourRow],
    aliases: &BTreeMap<String, String>,
) -> Result<Option<Frame>, ImportError> {
    for shape in SHAPES {
        if let Some(f) = read_shape(tree, skin, shape, colours, aliases)? {
            return Ok(Some(f));
        }
    }
    Ok(None)
}

fn read_shape(
    tree: &Tree,
    skin: &Ini,
    shape: &'static Shape,
    colours: &[ColourRow],
    aliases: &BTreeMap<String, String>,
) -> Result<Option<Frame>, ImportError> {
    let mut frame: Option<Frame> = None;
    for (prefix, state) in STATES {
        let mut files = BTreeSet::new();
        let pieces = match state_pieces(tree, skin, shape, prefix, colours, aliases, &mut files)? {
            Ok(p) => p,
            Err(missing) => {
                if let Some(f) = frame.as_mut().filter(|_| missing.len() < 8) {
                    f.skipped
                        .push((state, format!("missing {}", missing.join(", "))));
                }
                continue;
            }
        };
        let (img, insets) = bitmap::compose("window frame", &pieces)?;
        match frame.as_mut() {
            None if state == RegionState::Normal => {
                frame = Some(Frame {
                    shape,
                    insets,
                    images: BTreeMap::from([(state, img)]),
                    files,
                    skipped: Vec::new(),
                });
            }
            None => {}
            Some(f) if f.insets == insets => {
                f.images.insert(state, img);
                f.files.extend(files);
            }
            Some(f) => f.skipped.push((
                state,
                "its pieces differ in size from the normal state's".to_owned(),
            )),
        }
    }
    Ok(frame)
}
