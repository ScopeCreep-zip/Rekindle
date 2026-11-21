//! The INI era's window frame (plan E7.7.3 item 2): the skin file's frame
//! sections in one of [`SHAPES`], the first whose normal state is
//! complete, one image per complete state ([`super::STATES`]), each piece
//! shifted by its section's `<State>.ColorGroup` and keyed.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, RegionState};

use super::super::xfire_xml::{bitmap, ColourRow};
use rekindle_types::dto::skin::{SkinWindow, TileAxis, WindowLayout};

use super::super::views;
use super::super::{ImportError, Tree};
use super::ini::{Ini, Section};
use super::{resolve, STATES};
use crate::shape;

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
    /// The normal state's nine pieces, uncomposed: the main window's
    /// frame tiles (plan SK0 S1).
    pub pieces: [RgbaImage; 9],
    /// The windows laid beneath the frame pieces ([`Background`]).
    pub backgrounds: Vec<Background>,
    /// The views' insets (left, right) inside the frame (plan SK0-L1,
    /// [`views`]): `[MainWindow]`'s window `Name=TabbedControl` stretched
    /// both ways, its `X` and `IndentRight` (Alloy `Window12` `X=20
    /// IndentRight=19`); `None` without one or with a placement that is
    /// not a pixel count.
    pub views: Option<(u32, u32)>,
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
        if let Some(mut f) = read_shape(tree, skin, shape, colours, aliases)? {
            if let Some(main) = skin.section("MainWindow") {
                f.backgrounds = backgrounds(tree, skin, main, colours, aliases)?;
                f.views = views_insets(main);
            }
            return Ok(Some(f));
        }
    }
    Ok(None)
}

/// [`Frame::views`] from `[MainWindow]`.
fn views_insets(main: &Section) -> Option<(u32, u32)> {
    main.entries.iter().find_map(|entry| {
        let index = entry.key.strip_prefix("Window")?.strip_suffix(".Name")?;
        if !entry.value.trim().eq_ignore_ascii_case("TabbedControl") {
            return None;
        }
        let get = |key: &str| main.get(&format!("Window{index}.{key}"));
        if get("ResizeX") != Some("100") || get("ResizeY") != Some("100") {
            return None;
        }
        if get("JustX").is_some_and(|j| !j.trim().eq_ignore_ascii_case("LEFT")) {
            return None;
        }
        let num = |key: &str| get(key).map_or(Some(0), |v| v.trim().parse::<u32>().ok());
        Some((num("X")? + num("IndentLeft")?, num("IndentRight")?))
    })
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
                    pieces,
                    backgrounds: Vec::new(),
                    views: None,
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

/// A `[MainWindow]` window stretched to the window less its indents whose
/// section lays one tiled `Graphics.middle` image (Alloy's `MAINBG`:
/// `Alloy.ini` `Window19` `Z=-1 X=5 Y=5 IndentRight=5 IndentBottom=5`,
/// `[MAINBG] Graphics.middle=MAINBG`). Xfire draws it beneath the frame
/// pieces, so it fills their keyed holes: it is part of the window's
/// outline (plan SK0 S1e), a decoration (its `Z`, a negative one as 0).
#[derive(Debug)]
pub struct Background {
    pub name: String,
    pub image: RgbaImage,
    /// `X`, `Y`, `IndentRight`, `IndentBottom`, px.
    pub at: [u16; 4],
    pub z: u8,
}

fn backgrounds(
    tree: &Tree,
    skin: &Ini,
    main: &Section,
    colours: &[ColourRow],
    aliases: &BTreeMap<String, String>,
) -> Result<Vec<Background>, ImportError> {
    let mut out = Vec::new();
    for entry in &main.entries {
        let Some(index) = entry
            .key
            .strip_prefix("Window")
            .and_then(|k| k.strip_suffix(".Name"))
        else {
            continue;
        };
        let get = |key: &str| main.get(&format!("Window{index}.{key}"));
        let num = |key: &str| get(key).map_or(Some(0), |v| v.trim().parse::<u16>().ok());
        let stretched = get("ResizeX") == Some("100") && get("ResizeY") == Some("100");
        let at = [num("X"), num("Y"), num("IndentRight"), num("IndentBottom")];
        let (true, [Some(left), Some(top), Some(right), Some(bottom)]) = (stretched, at) else {
            continue;
        };
        let Some(section) = skin.section(entry.value.trim()) else {
            continue;
        };
        let sides = ["left", "right", "top", "bottom"];
        let (Some(value), false) = (
            section.get("Graphics.middle"),
            sides
                .iter()
                .any(|side| section.get(&format!("Graphics.{side}")).is_some()),
        ) else {
            continue;
        };
        let (file, bytes) = resolve(tree, value, aliases)?;
        let mut image = bitmap::decode(file, bytes)?;
        let by = section
            .get("Graphics.ColorGroup")
            .and_then(|group| colours.iter().find(|row| row.name == group))
            .map_or([0; 3], |row| row.value);
        bitmap::shift(&mut image, by);
        let depth = get("Z")
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(0);
        out.push(Background {
            name: entry.value.trim().to_ascii_lowercase(),
            image,
            at: [left, top, right, bottom],
            z: u8::try_from(depth.clamp(0, 255)).unwrap_or(0),
        });
    }
    Ok(out)
}

/// The buddy list's layout: the nine pieces, then each background tiled
/// over the window less its indents as a decoration.
///
/// # Errors
/// Pieces that do not compose, or an image too large for a tile.
pub fn main_layout(f: &Frame) -> Result<(WindowLayout, BTreeMap<String, Vec<u8>>), ImportError> {
    let window = SkinWindow::Main;
    let mut pieces = f.pieces.clone();
    if let Some(insets) = f.views {
        let [_, _, _, l, _, r, _, _, _] = &mut pieces;
        views::to_views_edge(l, r, insets);
    }
    let (mut layout, mut files) = shape::from_nine(window, &pieces)?;
    for bg in &f.backgrounds {
        let [left, top, right, bottom] = bg.at;
        let axes = (
            TileAxis::Repeat {
                start: left,
                end: right,
            },
            TileAxis::Repeat {
                start: top,
                end: bottom,
            },
        );
        let name = format!("bg-{}", bg.name);
        let Some((mut tile, png)) = shape::tile(window, &name, &bg.image, axes, true)? else {
            continue;
        };
        tile.decoration = Some(bg.z);
        files.insert(tile.image.clone(), png);
        layout.tiles.push(tile);
    }
    Ok((layout, files))
}
