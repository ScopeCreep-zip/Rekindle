//! The pack-laid title strip (plan SK1-S1, lead decision (1)): the
//! helper's refusals of a [`TitleLayout`] and its served rules.
//!
//! Xfire's strip is pack tiles at the pack's pixel coordinates with no
//! title text (research `sk1r/research.md` §1): the logo, the window-name
//! field and the minimise / maximise / close `PushButton`s, each button
//! with its Normal / Hover / Down images. Here each part is placed in the
//! window (`.app-frame` is the containing block), a button draws its
//! region's images at their own size with no glyph over them (the glyph
//! stays in the DOM, hidden, for the app's own strip), the accessible
//! name stays the button's `aria-label`, and an invisible 24 × 24 hit area
//! is centred on the art (WCAG 2.2 SC 2.5.8). The strip is the drag
//! region; the buttons are not.
//!
//! The rules (each a refusal of the whole package):
//! 1. at most one part per role, and a close button (a laid strip never
//!    hides the only way to close the window);
//! 2. every part lies inside the strip (`y + height ≤ bottom`), the strip
//!    at most [`MAX_BOTTOM`] px, each part at most [`MAX_PART`] px a side;
//! 3. `x` is `Start` or `End`; only the name field may stretch (`Repeat`,
//!    width 0);
//! 4. the logo and each button draw a package image at exactly the part's
//!    size (never stretched); a button's hover and down images too, and
//!    nothing else has state images;
//! 5. the name field has a face: a font name of letters, digits, spaces
//!    and hyphens (it is written into CSS), 8–32 px, a colour (when set)
//!    of `#rrggbb`, and its 3-slice art (when set) three package images
//!    at the part's height.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use rekindle_types::dto::skin::{
    RegionState, SkinError, SkinManifest, SkinRule, SkinWindow, TileAxis,
};
use rekindle_types::dto::skin_title::{TitleLayout, TitlePart, TitleRole};

use crate::regions::css::{dimensions, href};

/// The lowest a strip may reach into the window, px.
pub const MAX_BOTTOM: u16 = 256;
/// The largest side of a part, px.
pub const MAX_PART: u16 = 256;
/// The longest name-field font name.
const MAX_FONT: usize = 64;

fn bad(field: String, expected: &str) -> SkinError {
    SkinError::new(
        field.clone(),
        SkinRule::BadValue {
            field,
            expected: expected.to_owned(),
        },
    )
}

fn window_name(window: SkinWindow) -> &'static str {
    super::css::window_attr(window)
}

fn check_part(
    field: &str,
    p: &TitlePart,
    bottom: u16,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<(), SkinError> {
    let at = |k: &str| format!("{field}.{k}");
    if p.width > MAX_PART || p.height > MAX_PART || p.height == 0 {
        return Err(bad(
            at("height"),
            "a part 1-256 px tall and at most 256 px wide",
        ));
    }
    if u32::from(p.y) + u32::from(p.height) > u32::from(bottom) {
        return Err(bad(at("y"), "a part inside the strip"));
    }
    match (p.x, p.role) {
        (TileAxis::Start { .. } | TileAxis::End { .. }, _) => {}
        (TileAxis::Repeat { .. }, TitleRole::Name) if p.width == 0 => {}
        _ => {
            return Err(bad(
                at("x"),
                "start or end (a stretching name field: repeat, width 0)",
            ))
        }
    }
    let sized =
        |path: &str| dimensions(path, images) == Some((u32::from(p.width), u32::from(p.height)));
    let states_ok = p.states.iter().all(|(s, path)| {
        p.role.is_button() && matches!(s, RegionState::Hover | RegionState::Down) && sized(path)
    });
    if !states_ok {
        return Err(bad(
            at("states"),
            "a button's hover and down images at the part's size",
        ));
    }
    if p.role != TitleRole::Name {
        if p.text.is_some() || !p.image.as_deref().is_some_and(sized) {
            return Err(bad(
                at("image"),
                "a package image at exactly the part's size",
            ));
        }
        return Ok(());
    }
    let hex = |c: &str| {
        c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|d| d.is_ascii_hexdigit())
    };
    let face_ok = p.text.as_ref().is_some_and(|t| {
        t.colour.as_deref().is_none_or(hex)
            && (8..=32).contains(&t.size)
            && !t.font.trim().is_empty()
            && t.font.len() <= MAX_FONT
            && t.font
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
    });
    if p.image.is_some() || !face_ok {
        return Err(bad(
            at("text"),
            "a font of letters, digits, spaces and hyphens, 8-32 px, a #rrggbb colour",
        ));
    }
    let slices_ok = p.slices.as_ref().is_none_or(|s| {
        [&s.left, &s.middle, &s.right].into_iter().all(|path| {
            dimensions(path, images).is_some_and(|(w, h)| w > 0 && h == u32::from(p.height))
        })
    });
    if !slices_ok {
        return Err(bad(
            at("slices"),
            "three package images at the part's height",
        ));
    }
    Ok(())
}

/// Refuses a manifest whose title layouts break the rules above.
///
/// # Errors
/// The first rule a title layout breaks.
pub fn check(manifest: &SkinManifest, images: &BTreeMap<String, Vec<u8>>) -> Result<(), SkinError> {
    for (&window, layout) in &manifest.windows {
        let Some(title) = &layout.title else { continue };
        let field = format!("windows.{}.title", window_name(window));
        if title.bottom > MAX_BOTTOM {
            return Err(bad(format!("{field}.bottom"), "at most 256 px"));
        }
        let mut seen = BTreeSet::new();
        for (i, p) in title.parts.iter().enumerate() {
            if !seen.insert(p.role) {
                return Err(bad(format!("{field}.parts"), "one part per role"));
            }
            check_part(&format!("{field}.parts.{i}"), p, title.bottom, images)?;
        }
        if !seen.contains(&TitleRole::Close) {
            return Err(bad(format!("{field}.parts"), "a close button"));
        }
    }
    Ok(())
}

fn x_decls(x: TileAxis) -> String {
    match x {
        TileAxis::Start { offset } => format!("left: {offset}px;"),
        TileAxis::End { offset } => format!("right: {offset}px;"),
        TileAxis::Repeat { start, end } => format!("left: {start}px; right: {end}px;"),
        TileAxis::Centre => String::new(),
    }
}

fn placed(p: &TitlePart) -> String {
    let w = if p.width == 0 {
        String::new()
    } else {
        format!(" width: {}px;", p.width)
    };
    format!(
        "display: block; position: absolute; {} top: {}px;{w} height: {}px; margin: 0; padding: 0;",
        x_decls(p.x),
        p.y,
        p.height
    )
}

fn button(
    out: &mut String,
    r: &str,
    p: &TitlePart,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<()> {
    let sel = format!("{r} .xfire-titlebar-btn-{}", p.role.key());
    let size = format!("{}px {}px", p.width, p.height);
    let normal = href(base, p.image.as_deref()?, images)?;
    let _ = writeln!(
        out,
        "  {sel} {{\n    {} border: 0; border-radius: 0; background: url(\"{normal}\") 0 0 / {size} no-repeat;\n  }}\n  {sel} > .xfire-titlebar-glyph {{\n    display: none;\n  }}\n  {sel}::before {{\n    content: \"\";\n    position: absolute;\n    left: 50%;\n    top: 50%;\n    width: max(100%, 24px);\n    height: max(100%, 24px);\n    translate: -50% -50%;\n  }}",
        placed(p)
    );
    for (state, pseudo) in [
        (RegionState::Hover, ":hover"),
        (RegionState::Down, ":active"),
    ] {
        if let Some(img) = p.states.get(&state) {
            let h = href(base, img, images)?;
            let _ = writeln!(
                out,
                "  {sel}{pseudo} {{\n    background-image: url(\"{h}\");\n  }}"
            );
        }
    }
    Some(())
}

/// The name field: its face, its pack colour, and its 3-slice art (the
/// ends in the transparent side borders at their own size, the middle
/// repeated in the padding box between them, never under the ends).
fn name(
    out: &mut String,
    root: &str,
    p: &TitlePart,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<()> {
    let t = p.text.as_ref()?;
    let weight = if t.bold { "bold" } else { "normal" };
    let colour = t
        .colour
        .as_deref()
        .map_or(String::new(), |c| format!(" color: {c};"));
    let mut art = String::new();
    let [mut pad_l, mut pad_r] = t.inset;
    if let Some(s) = &p.slices {
        let (lw, _) = dimensions(&s.left, images)?;
        let (rw, _) = dimensions(&s.right, images)?;
        let (mw, _) = dimensions(&s.middle, images)?;
        let (left, middle, right) = (
            href(base, &s.left, images)?,
            href(base, &s.middle, images)?,
            href(base, &s.right, images)?,
        );
        let height = p.height;
        pad_l = pad_l.saturating_sub(u16::try_from(lw).ok()?);
        pad_r = pad_r.saturating_sub(u16::try_from(rw).ok()?);
        let _ = write!(
            art,
            " border-style: solid; border-color: transparent; border-width: 0 {rw}px 0 {lw}px; background: url(\"{left}\") left top / {lw}px {height}px no-repeat border-box border-box, url(\"{right}\") right top / {rw}px {height}px no-repeat border-box border-box, url(\"{middle}\") 0 0 / {mw}px {height}px repeat-x padding-box padding-box;"
        );
    }
    let _ = writeln!(
        out,
        "  {root} .xfire-titlebar-text {{\n    {} box-sizing: border-box; padding: 0 {pad_r}px 0 {pad_l}px; font-family: \"{}\"; font-size: {}px; font-weight: {weight}; line-height: {}px;{colour}{art} white-space: nowrap; overflow: hidden; text-overflow: ellipsis;\n  }}",
        placed(p),
        t.font.trim(),
        t.size,
        p.height
    );
    Some(())
}

/// The rules laying `title` in every window `root` selects. `top` is the
/// content's top edge in the window (the strip row above it is the drag
/// region). `None` for an image `images` lacks (never for a validated
/// package).
#[must_use]
pub fn title_css(
    root: &str,
    title: &TitleLayout,
    top: u16,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<String> {
    let mut out = String::new();
    let row = title.bottom.saturating_sub(top);
    let _ = writeln!(
        out,
        "  {root} {{\n    position: relative;\n  }}\n  {root} .xfire-titlebar {{\n    height: {row}px;\n    padding: 0;\n  }}\n  {root} .xfire-titlebar-btn,\n  {root} .xfire-titlebar-text,\n  {root} .xfire-titlebar-spacer {{\n    display: none;\n  }}"
    );
    for p in &title.parts {
        match p.role {
            // The pack's logo box only places the shell's inline brand
            // glyph (`SkinApplied::title_logo`), never the pack's Xfire
            // logo (design `br`, `skin_tree::brand`).
            TitleRole::Logo => {
                let _ = writeln!(
                    out,
                    "  {root} .xfire-titlebar-logo {{\n    {}\n  }}",
                    placed(p)
                );
            }
            TitleRole::Name => name(&mut out, root, p, images, base)?,
            _ => button(&mut out, root, p, images, base)?,
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "title_tests.rs"]
mod tests;
