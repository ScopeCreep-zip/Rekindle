//! The 39 of Webamp's 228 sprites that map (d1 §2.3; coordinates from
//! `webamp/js/skinSprites.ts:611-622,723-754,760-838`), the frame and
//! titlebar built from GEN.BMP, and GENEX's 22 colour pixels (d1 §2.4,
//! `skinParserUtils.ts:188-261`).

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, RegionState};

use super::super::ImportError;

/// The region state a sprite draws: Webamp's `_DEPRESSED` and `_PRESSED`
/// sprites are `down`, every other one the normal image.
#[must_use]
pub fn state(sprite: &Sprite) -> RegionState {
    if sprite.name.ends_with("_DEPRESSED") || sprite.name.ends_with("_PRESSED") {
        RegionState::Down
    } else {
        RegionState::Normal
    }
}

/// One sprite: sheet (lower-case file), name, x, y, width, height, and the
/// Rekindle region it maps to (d1's proposed id).
#[derive(Debug, Clone, Copy)]
pub struct Sprite {
    pub sheet: &'static str,
    pub name: &'static str,
    pub rect: (u32, u32, u32, u32),
    pub region: &'static str,
}

const fn s(
    sheet: &'static str,
    name: &'static str,
    rect: (u32, u32, u32, u32),
    region: &'static str,
) -> Sprite {
    Sprite {
        sheet,
        name,
        rect,
        region,
    }
}

const GEN: &str = "gen.bmp";
const GENEX: &str = "genex.bmp";
const TITLEBAR: &str = "titlebar.bmp";
const FRAME: &str = "window.frame";
const TITLE: &str = "titlebar";

/// The mapped sprites, d1 §2.3 order.
pub const SPRITES: [Sprite; 39] = [
    s(GEN, "GEN_TOP_LEFT_SELECTED", (0, 0, 25, 20), FRAME),
    s(GEN, "GEN_TOP_LEFT_END_SELECTED", (26, 0, 25, 20), TITLE),
    s(GEN, "GEN_TOP_CENTER_FILL_SELECTED", (52, 0, 25, 20), TITLE),
    s(GEN, "GEN_TOP_RIGHT_END_SELECTED", (78, 0, 25, 20), TITLE),
    s(
        GEN,
        "GEN_TOP_LEFT_RIGHT_FILL_SELECTED",
        (104, 0, 25, 20),
        FRAME,
    ),
    s(GEN, "GEN_TOP_RIGHT_SELECTED", (130, 0, 25, 20), FRAME),
    s(GEN, "GEN_TOP_LEFT", (0, 21, 25, 20), FRAME),
    s(GEN, "GEN_TOP_LEFT_END", (26, 21, 25, 20), TITLE),
    s(GEN, "GEN_TOP_CENTER_FILL", (52, 21, 25, 20), TITLE),
    s(GEN, "GEN_TOP_RIGHT_END", (78, 21, 25, 20), TITLE),
    s(GEN, "GEN_TOP_LEFT_RIGHT_FILL", (104, 21, 25, 20), FRAME),
    s(GEN, "GEN_TOP_RIGHT", (130, 21, 25, 20), FRAME),
    s(GEN, "GEN_BOTTOM_LEFT", (0, 42, 125, 14), FRAME),
    s(GEN, "GEN_BOTTOM_RIGHT", (0, 57, 125, 14), FRAME),
    s(GEN, "GEN_BOTTOM_FILL", (127, 72, 25, 14), FRAME),
    s(GEN, "GEN_MIDDLE_LEFT", (127, 42, 11, 29), FRAME),
    s(GEN, "GEN_MIDDLE_LEFT_BOTTOM", (158, 42, 11, 24), FRAME),
    s(GEN, "GEN_MIDDLE_RIGHT", (139, 42, 8, 29), FRAME),
    s(GEN, "GEN_MIDDLE_RIGHT_BOTTOM", (170, 42, 8, 24), FRAME),
    s(
        TITLEBAR,
        "MAIN_MINIMIZE_BUTTON",
        (9, 0, 9, 9),
        "titlebar.btn.minimize",
    ),
    s(
        TITLEBAR,
        "MAIN_MINIMIZE_BUTTON_DEPRESSED",
        (9, 9, 9, 9),
        "titlebar.btn.minimize",
    ),
    s(
        TITLEBAR,
        "MAIN_CLOSE_BUTTON",
        (18, 0, 9, 9),
        "titlebar.btn.close",
    ),
    s(
        TITLEBAR,
        "MAIN_CLOSE_BUTTON_DEPRESSED",
        (18, 9, 9, 9),
        "titlebar.btn.close",
    ),
    s(
        GENEX,
        "BUTTON_BACKGROUND_LEFT_UNPRESSED",
        (0, 0, 4, 15),
        "button.secondary",
    ),
    s(
        GENEX,
        "BUTTON_BACKGROUND_CENTER_UNPRESSED",
        (4, 0, 39, 15),
        "button.secondary",
    ),
    s(
        GENEX,
        "BUTTON_BACKGROUND_RIGHT_UNPRESSED",
        (43, 0, 4, 15),
        "button.secondary",
    ),
    s(
        GENEX,
        "BUTTON_BACKGROUND_PRESSED",
        (0, 1, 47, 15),
        "button.secondary",
    ),
    s(
        GENEX,
        "SCROLL_UP_UNPRESSED",
        (0, 31, 14, 14),
        "scrollbar.button-up",
    ),
    s(
        GENEX,
        "SCROLL_DOWN_UNPRESSED",
        (14, 31, 14, 14),
        "scrollbar.button-down",
    ),
    s(
        GENEX,
        "SCROLL_UP_PRESSED",
        (28, 31, 14, 14),
        "scrollbar.button-up",
    ),
    s(
        GENEX,
        "SCROLL_DOWN_PRESSED",
        (42, 31, 14, 14),
        "scrollbar.button-down",
    ),
    s(
        GENEX,
        "SCROLL_LEFT_UNPRESSED",
        (0, 45, 14, 14),
        "scrollbar.button-left",
    ),
    s(
        GENEX,
        "SCROLL_RIGHT_UNPRESSED",
        (14, 45, 14, 14),
        "scrollbar.button-right",
    ),
    s(
        GENEX,
        "SCROLL_LEFT_PRESSED",
        (28, 45, 14, 14),
        "scrollbar.button-left",
    ),
    s(
        GENEX,
        "SCROLL_RIGHT_PRESSED",
        (42, 45, 14, 14),
        "scrollbar.button-right",
    ),
    s(
        GENEX,
        "VERTICAL_SCROLL_HANDLE_UNPRESSED",
        (56, 31, 14, 28),
        "scrollbar.thumb",
    ),
    s(
        GENEX,
        "VERTICAL_SCROLL_HANDLE_PRESSED",
        (70, 31, 14, 28),
        "scrollbar.thumb",
    ),
    s(
        GENEX,
        "HORIZONTAL_SCROLL_HANDLE_UNPRESSED",
        (84, 31, 28, 14),
        "scrollbar.thumb-horizontal",
    ),
    s(
        GENEX,
        "HORIZONTAL_SCROLL_HANDLE_PRESSED",
        (84, 45, 28, 14),
        "scrollbar.thumb-horizontal",
    ),
];

/// GENEX's colour pixels: (x on row 0, meaning, what it becomes).
pub const GENEX_COLOURS: [(u32, &str, Target); 22] = [
    (48, "item background", Target::Tokens(&["bg.input"])),
    (50, "item foreground", Target::NoRole("input.text")),
    (52, "window background", Target::Tokens(&["bg.panel"])),
    (54, "button text", Target::NoRole("button.secondary.text")),
    (
        56,
        "window text",
        Target::Tokens(&["text.primary", "titlebar.text"]),
    ),
    (
        58,
        "dividers and sunken borders",
        Target::Tokens(&["border"]),
    ),
    (60, "playlist selection", Target::NoRole("selection.bg")),
    (
        62,
        "listview header background",
        Target::NoRole("list.group-header.bg"),
    ),
    (
        64,
        "listview header text",
        Target::NoRole("list.group-header.text"),
    ),
    (
        66,
        "header frame top",
        Target::NoRole("list.group-header bevel"),
    ),
    (
        68,
        "header frame middle",
        Target::NoRole("list.group-header bevel"),
    ),
    (
        70,
        "header frame bottom",
        Target::NoRole("list.group-header bevel"),
    ),
    (
        72,
        "header dead area",
        Target::Dropped("no dead area in Rekindle lists"),
    ),
    (74, "scrollbar colour 1", Target::NoRole("scrollbar.thumb")),
    (76, "scrollbar colour 2", Target::NoRole("scrollbar.track")),
    (
        78,
        "pressed scrollbar colour 1",
        Target::NoRole("scrollbar.thumb.active"),
    ),
    (
        80,
        "pressed scrollbar colour 2",
        Target::NoRole("scrollbar.track.active"),
    ),
    (
        82,
        "scrollbar dead area",
        Target::NoRole("scrollbar.corner"),
    ),
    (
        84,
        "list text highlighted",
        Target::NoRole("list.hover.text"),
    ),
    (
        86,
        "list background highlighted",
        Target::NoRole("list.hover.bg"),
    ),
    (
        88,
        "list text selected",
        Target::Dropped("PLEDIT.TXT owns selection"),
    ),
    (
        90,
        "list background selected",
        Target::Dropped("PLEDIT.TXT owns selection"),
    ),
];

/// What a Winamp colour became.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Tokens(&'static [&'static str]),
    NoRole(&'static str),
    Dropped(&'static str),
}

/// The sprite `name`'s pixels from its sheet.
///
/// # Errors
/// [`ImportError::Image`] when the sheet is too small for it.
pub fn crop(sheet: &RgbaImage, sprite: &Sprite) -> Result<RgbaImage, ImportError> {
    let (x, y, w, h) = sprite.rect;
    if x + w > sheet.width() || y + h > sheet.height() {
        return Err(ImportError::Image {
            file: sprite.sheet.to_owned(),
            detail: format!(
                "{} at {x},{y} {w}x{h} is outside the {}x{} sheet",
                sprite.name,
                sheet.width(),
                sheet.height()
            ),
        });
    }
    Ok(RgbaImage::from_fn(w, h, |cx, cy| {
        *sheet.get_pixel(x + cx, y + cy)
    }))
}

/// The cropped sprite `name` from `sprites` (decoded in [`SPRITES`] order).
fn get<'a>(sprites: &'a [(Sprite, RgbaImage)], name: &str) -> &'a RgbaImage {
    &sprites
        .iter()
        .find(|(s, _)| s.name == name)
        .unwrap_or(&sprites[0])
        .1
}

fn blit(out: &mut RgbaImage, src: &RgbaImage, sx: u32, x0: u32, y0: u32, w: u32) {
    for y in 0..src.height() {
        for x in 0..w.min(src.width().saturating_sub(sx)) {
            out.put_pixel(x0 + x, y0 + y, *src.get_pixel(sx + x, y));
        }
    }
}

/// The frame's nine-slice insets: the top row's 25 px corners, GEN's 20 px
/// top, and the bottom row (24 px of edge stacked on the 14 px bar).
pub const FRAME_INSETS: Insets = Insets {
    top: 20,
    right: 25,
    bottom: 38,
    left: 25,
};

/// `window.frame` for one state (`suffix` = `"_SELECTED"` or `""`), on
/// `ground` (GENEX's window background, or transparent). Import transforms,
/// recorded in the NOTICE: the close cell baked into `GEN_TOP_RIGHT`
/// (right 2, top 3, 9×9) is repainted with the fill's pixels; the 125 px
/// bottom bars are cut to their outer 25 px (the grip is decoration).
pub fn frame(sprites: &[(Sprite, RgbaImage)], suffix: &str, ground: [u8; 4]) -> RgbaImage {
    let top = |n: &str| get(sprites, &format!("GEN_TOP_{n}{suffix}")).clone();
    let fill = top("LEFT_RIGHT_FILL");
    let mut right = top("RIGHT");
    for y in 3..12 {
        for x in 14..23 {
            right.put_pixel(x, y, *fill.get_pixel(x % fill.width(), y));
        }
    }
    let mut out = RgbaImage::from_pixel(75, 87, Rgba(ground));
    blit(&mut out, &top("LEFT"), 0, 0, 0, 25);
    blit(&mut out, &fill, 0, 25, 0, 25);
    blit(&mut out, &right, 0, 50, 0, 25);
    blit(&mut out, get(sprites, "GEN_MIDDLE_LEFT"), 0, 0, 20, 11);
    blit(&mut out, get(sprites, "GEN_MIDDLE_RIGHT"), 0, 67, 20, 8);
    blit(
        &mut out,
        get(sprites, "GEN_MIDDLE_LEFT_BOTTOM"),
        0,
        0,
        49,
        11,
    );
    blit(
        &mut out,
        get(sprites, "GEN_MIDDLE_RIGHT_BOTTOM"),
        0,
        67,
        49,
        8,
    );
    blit(&mut out, get(sprites, "GEN_BOTTOM_LEFT"), 0, 0, 73, 25);
    blit(&mut out, get(sprites, "GEN_BOTTOM_FILL"), 0, 25, 73, 25);
    blit(&mut out, get(sprites, "GEN_BOTTOM_RIGHT"), 100, 50, 73, 25);
    out
}

/// The GENEX button: its slot, its three unpressed pieces (left, centre,
/// right) and their nine-slice insets (d1 §2.3: l4 r4 per the SDK, t/b 4).
pub const BUTTON: &str = "button.secondary";
pub const BUTTON_PARTS: [&str; 3] = [
    "BUTTON_BACKGROUND_LEFT_UNPRESSED",
    "BUTTON_BACKGROUND_CENTER_UNPRESSED",
    "BUTTON_BACKGROUND_RIGHT_UNPRESSED",
];
pub const BUTTON_INSETS: Insets = Insets {
    top: 4,
    right: 4,
    bottom: 4,
    left: 4,
};
/// The token the button's `textBacking` names: GENEX's window background,
/// the ground the walk already holds the label (`text.base`) at 4.5:1 on.
/// An opaque backing covers the image inside the button's border, so the
/// label is measured against it alone.
pub const BUTTON_BACKING: &str = "bg.panel";

/// The button's normal image: its left, centre and right pieces side by
/// side (4 + 39 + 4 × 15), one nine-slice whose insets are the end caps.
/// `None` when GENEX did not hold them.
pub fn button(sprites: &[(Sprite, RgbaImage)]) -> Option<RgbaImage> {
    let parts: Vec<&RgbaImage> = BUTTON_PARTS
        .iter()
        .map(|n| sprites.iter().find(|(s, _)| s.name == *n).map(|(_, i)| i))
        .collect::<Option<_>>()?;
    let width = parts.iter().map(|p| p.width()).sum();
    let height = parts.iter().map(|p| p.height()).max().unwrap_or(0);
    let mut out = RgbaImage::new(width, height);
    let mut x0 = 0;
    for part in parts {
        blit(&mut out, part, 0, x0, 0, part.width());
        x0 += part.width();
    }
    Some(out)
}

/// The titlebar's nine-slice insets: the two 25 px title caps.
pub const TITLE_INSETS: Insets = Insets {
    top: 0,
    right: 25,
    bottom: 0,
    left: 25,
};

/// `titlebar` for one state: left cap, centre fill, right cap.
pub fn titlebar(sprites: &[(Sprite, RgbaImage)], suffix: &str) -> RgbaImage {
    let top = |n: &str| get(sprites, &format!("GEN_TOP_{n}{suffix}"));
    let mut out = RgbaImage::new(75, 20);
    blit(&mut out, top("LEFT_END"), 0, 0, 0, 25);
    blit(&mut out, top("CENTER_FILL"), 0, 25, 0, 25);
    blit(&mut out, top("RIGHT_END"), 0, 50, 0, 25);
    out
}

/// The most frequent opaque colour of `img` (ties: the lowest RGB), the
/// titlebar's `textBacking` (d1 §2.3).
pub fn dominant(img: &RgbaImage) -> Option<[u8; 3]> {
    let mut counts = std::collections::BTreeMap::new();
    for p in img.pixels().filter(|p| p.0[3] == 255) {
        *counts.entry([p.0[0], p.0[1], p.0[2]]).or_insert(0u32) += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(c, _)| c)
}
