//! A window layout from a nine-piece frame (plan SK0 S1): the pieces
//! every Xfire importer already reads (`bitmap::NINE` order), laid the way
//! the packs lay them. Space Cowboy's `MainWindow.xml:2-11` is the rule
//! spelled out: `TopLeft` at 0,0; `Top` from `TopLeft.RIGHT` with
//! `IndentRight="TopRight.width"`, `ResizeX="100"`; `TopRight`
//! `JustX="RIGHT"`; `LeftEdge` from `TopLeft.BOTTOM` with
//! `IndentBottom="BottomLeft.height"`, `ResizeY="100"`; the bottom row
//! `JustY="BOTTOM"` the same way. A resizing tile repeats its image at its
//! source size (the `Tile` element is Xfire's tiling; research §7.2), so
//! the middles are [`TileAxis::Repeat`]. The centre piece is the content
//! ground, not a frame tile (the content rectangle is always opaque).

use std::collections::BTreeMap;

use rekindle_image::{encode_png, DynamicImage, RgbaImage};
use rekindle_types::dto::skin::{FrameTile, Insets, SkinWindow, TileAxis, WindowLayout};

use super::Mask;
use crate::import::ImportError;

/// The piece names, in `bitmap::NINE` order (the centre is skipped).
const PIECES: [&str; 9] = [
    "top-left",
    "top",
    "top-right",
    "left",
    "centre",
    "right",
    "bottom-left",
    "bottom",
    "bottom-right",
];

/// The package path of `window`'s `piece` tile.
#[must_use]
pub fn tile_path(window: SkinWindow, piece: &str) -> String {
    let w = match window {
        SkinWindow::Main => "main",
        SkinWindow::Chat => "chat",
    };
    format!("assets/frame/{w}/{piece}.png")
}

pub(crate) fn u16_of(file: &str, v: u32) -> Result<u16, ImportError> {
    u16::try_from(v).map_err(|_| ImportError::Image {
        file: file.to_owned(),
        detail: "a frame piece is too large".to_owned(),
    })
}

/// The frame tile of `img` as `window`'s `piece`, placed by `axes`, and
/// its PNG; `None` when the image has no opaque pixel (nothing to lay).
///
/// # Errors
/// An image too large for a tile, or one that does not encode.
pub fn tile(
    window: SkinWindow,
    piece: &str,
    img: &RgbaImage,
    axes: (TileAxis, TileAxis),
    overhang: bool,
) -> Result<Option<(FrameTile, Vec<u8>)>, ImportError> {
    let mask = Mask::of(img);
    if (0..img.height()).all(|r| mask.runs(r).is_empty()) {
        return Ok(None);
    }
    let path = tile_path(window, piece);
    let png =
        encode_png(&DynamicImage::ImageRgba8(img.clone())).map_err(|e| ImportError::Image {
            file: path.clone(),
            detail: e.to_string(),
        })?;
    Ok(Some((
        FrameTile {
            width: u16_of(&path, img.width())?,
            height: u16_of(&path, img.height())?,
            image: path,
            x: axes.0,
            y: axes.1,
            overhang,
            decoration: None,
        },
        png,
    )))
}

/// `window`'s layout from `pieces`, and the tile images it names.
///
/// # Errors
/// A piece too large for the layout, or one that does not encode.
pub fn from_nine(
    window: SkinWindow,
    pieces: &[RgbaImage; 9],
) -> Result<(WindowLayout, BTreeMap<String, Vec<u8>>), ImportError> {
    let w = |i: usize| pieces[i].width();
    let h = |i: usize| pieces[i].height();
    let start = TileAxis::Start { offset: 0 };
    let end = TileAxis::End { offset: 0 };
    let file = tile_path(window, "frame");
    let rep = |a: u32, b: u32| -> Result<TileAxis, ImportError> {
        Ok(TileAxis::Repeat {
            start: u16_of(&file, a)?,
            end: u16_of(&file, b)?,
        })
    };
    let axes: [(TileAxis, TileAxis); 9] = [
        (start, start),
        (rep(w(0), w(2))?, start),
        (end, start),
        (start, rep(h(0), h(6))?),
        (start, start),
        (end, rep(h(2), h(8))?),
        (start, end),
        (rep(w(6), w(8))?, end),
        (end, end),
    ];
    let mut tiles = Vec::new();
    let mut files = BTreeMap::new();
    for (i, (x, y)) in axes.into_iter().enumerate() {
        if i == 4 {
            continue;
        }
        if let Some((t, png)) = tile(window, PIECES[i], &pieces[i], (x, y), false)? {
            files.insert(t.image.clone(), png);
            tiles.push(t);
        }
    }
    // The content lies below the top row and above the bottom row (each as
    // tall as its tallest piece, so no corner reaches into it) and between
    // the side edges: a side's inset is its edge piece's width, never a
    // corner's (xfire-classic's 117 px bottom corners belong to the bottom
    // row, beneath the content; plan SK0 S2 finding), as the row stack's
    // content row (`import/xfire_xml/stack.rs`).
    let band = |a: usize, b: usize, c: usize| h(a).max(h(b)).max(h(c));
    let content = Insets {
        top: u16_of(&file, band(0, 1, 2))?,
        right: u16_of(&file, w(5))?,
        bottom: u16_of(&file, band(6, 7, 8))?,
        left: u16_of(&file, w(3))?,
    };
    let min_width = (w(0) + w(2))
        .max(w(6) + w(8))
        .max(u32::from(content.left) + u32::from(content.right));
    let min_height = (h(0) + h(6))
        .max(h(2) + h(8))
        .max(u32::from(content.top) + u32::from(content.bottom));
    Ok((
        WindowLayout {
            outsets: Insets {
                top: 0,
                right: 0,
                bottom: 0,
                left: 0,
            },
            content,
            min_width: u16_of(&file, min_width)?,
            min_height: u16_of(&file, min_height)?,
            tiles,
            header: None,
        },
        files,
    ))
}
