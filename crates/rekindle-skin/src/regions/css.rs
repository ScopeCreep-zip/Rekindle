//! Region CSS (plan E7.7.1 "Regions", validator step 7): two halves, so no
//! skin string ever names a selector.
//!
//! * [`app_css`], generated into `src/styles/generated/regions.css` by
//!   `cargo xtask gen-skin-registry` from the `[regions]` table of
//!   `src/styles/regions.toml`: in `@layer components`, after every app
//!   rule, each painted slot's elements and their state selectors read the
//!   slot's custom property into `border-image`. A state without its own
//!   image shows the normal one; with no skin region the property is unset
//!   and `border-image` computes to `none`.
//! * [`skin_css`], built per skin: `@layer skin { :root { … } }` setting
//!   those custom properties. A value is drawn from the numbers alone (the
//!   insets, the fit) and the image's BLAKE3 address under the Host's
//!   serving base, never its name in the package.
//!
//! A region is painted with `border-image` because its nine-slice is CSS
//! Backgrounds 3's (the insets at 1:1, shrinking in proportion in a small
//! box, [`super::sampled`]); `stretch` is a zero slice with `fill`, `tile`
//! the same with `repeat` (tiles centred). `border-image` is the property
//! the app's rules leave alone (`border: none` resets it, so the generated
//! rules come last), and the app's own `background` stays beneath the
//! image's transparent pixels. Painted slots are leaf controls, or a text
//! button whose children are its label and glyph-font icons (no
//! `.form-btn-secondary` holds an `<img>`), so their `image-rendering:
//! pixelated` (the F83 sampling's assumption) reaches no other image.
//!
//! * Under `forced-colors: active` every region rule's `border-image` is
//!   `none` (plan E7.7.2): a forced palette replaces the app's colours, and
//!   a skin bitmap left in place would draw the skin's colours over it
//!   (CSS Color Adjustment 1 §3.1 forces colours, not `border-image`).
//! * A scrollbar button has no length until a skin gives it one: its
//!   `height` (vertical) or `width` (horizontal) reads
//!   [`length_property`], which [`skin_css`] sets to the button image's
//!   length on that axis, so the arrow shows at 1:1 and a scrollbar
//!   without a skin keeps no buttons.
//! * A painted slot that carries text (`button.secondary`) gets its
//!   `textBacking` between the image and the label (plan E7.7.2b; CSS 2.1
//!   Appendix E): the element is `position: relative; isolation: isolate`
//!   and its `::before` (`position: absolute; inset: 0; z-index: -1`)
//!   paints [`backing_property`], which [`skin_css`] sets to the backing
//!   token's custom property, so it follows the active variant. Unset, the
//!   background is transparent. Under `forced-colors: active` the `::before`
//!   is not generated, so the user's `ButtonFace` is never covered.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_image::{read_header, ImageKind};
use rekindle_types::dto::skin::{RegionFit, RegionState, SkinManifest};

use super::slots::{slot_of, SLOTS};
use crate::images::extension;
use crate::limits::MAX_IMAGE_ALLOC;
use crate::registry::{TokenType, ENTRIES};

/// The scrollbar button slots and the side along their scroll axis.
const BUTTON_LENGTH: [(&str, &str); 4] = [
    ("scrollbar.button-up", "height"),
    ("scrollbar.button-down", "height"),
    ("scrollbar.button-left", "width"),
    ("scrollbar.button-right", "width"),
];

/// The side along the scroll axis of the button `slot`, if it is one.
fn button_side(slot: &str) -> Option<&'static str> {
    BUTTON_LENGTH
        .iter()
        .find(|(id, _)| *id == slot)
        .map(|(_, side)| *side)
}

/// The custom property holding a scrollbar button's length.
#[must_use]
pub fn length_property(slot: &str) -> String {
    format!("{}-length", property(slot, None))
}

/// The custom property holding a text slot's backing colour.
#[must_use]
pub fn backing_property(slot: &str) -> String {
    format!("{}-backing", property(slot, None))
}

/// The `::before` of a text slot's `elements` that paints the backing
/// under the label.
fn backing_rule(id: &str, elements: &[String]) -> Result<String, String> {
    if let Some(e) = elements.iter().find(|e| e.contains("::")) {
        return Err(format!(
            "`{id}`: `{e}` carries text, so it must be an element, not a pseudo-element"
        ));
    }
    let before: Vec<String> = elements.iter().map(|e| format!("{e}::before")).collect();
    Ok(format!(
        "  {} {{\n    content: \"\";\n    position: absolute;\n    inset: 0;\n    z-index: -1;\n    border-radius: inherit;\n    background-color: var({});\n    pointer-events: none;\n  }}\n",
        before.join(",\n  "),
        backing_property(id)
    ))
}

/// The custom property of `slot`, or of its `state` image.
#[must_use]
pub fn property(slot: &str, state: Option<RegionState>) -> String {
    let slug = slot.replace('.', "-");
    match state {
        None => format!("--skin-region-{slug}"),
        Some(s) => format!("--skin-region-{slug}--{}", s.key().to_ascii_lowercase()),
    }
}

/// The selector of `element` in `state`.
fn state_selector(element: &str, state: RegionState) -> Option<String> {
    let pseudo = element.contains("::");
    Some(match state {
        RegionState::Hover => format!("{element}:hover"),
        RegionState::Down => format!("{element}:active"),
        RegionState::Disabled if pseudo => format!("{element}:disabled"),
        RegionState::Disabled => format!("{element}:disabled, {element}[aria-disabled=\"true\"]"),
        RegionState::Focused if pseudo => return None,
        RegionState::Focused => format!("{element}:focus-visible"),
        // WebKit's scrollbar pseudo-class; elsewhere the attribute E7.7.2's
        // apply step sets on `:root` while the window is not focused, under
        // `:where()` so it adds no specificity and hover still wins.
        RegionState::WindowInactive if pseudo => format!("{element}:window-inactive"),
        RegionState::WindowInactive => format!(":where(:root[data-window-inactive]) {element}"),
        RegionState::Normal | RegionState::Ringing | RegionState::Talking | RegionState::Muted => {
            return None
        }
    })
}

/// Cascade order: a later state wins at equal specificity.
const ORDER: [RegionState; 5] = [
    RegionState::WindowInactive,
    RegionState::Hover,
    RegionState::Focused,
    RegionState::Down,
    RegionState::Disabled,
];

/// `src/styles/generated/regions.css` from the region map's `[regions]`
/// (slot id → the app elements it paints), with `header` as its comment.
///
/// # Errors
/// A map entry naming no painted slot, a painted slot the map lacks or
/// maps to no element, or a state a pseudo-element cannot take.
pub fn app_css(header: &str, map: &BTreeMap<String, Vec<String>>) -> Result<String, String> {
    for id in map.keys() {
        if !slot_of(id).is_some_and(|s| s.css) {
            return Err(format!("[regions] `{id}`: not a painted region slot"));
        }
    }
    // Every rule's selector list, for the forced-colors reset, and every
    // backing `::before`.
    let mut painted: Vec<String> = Vec::new();
    let mut backed: Vec<String> = Vec::new();
    let mut out = format!(
        "/* {header}\n *\n * Region slots (`rekindle-skin::regions`) on the elements\n * `src/styles/regions.toml` `[regions]` names; a skin sets the custom\n * properties in `@layer skin`. */\n@layer components {{\n"
    );
    for slot in SLOTS.iter().filter(|s| s.css) {
        let elements = map
            .get(slot.id)
            .filter(|e| !e.is_empty())
            .ok_or_else(|| format!("[regions] has no elements for `{}`", slot.id))?;
        let normal = property(slot.id, None);
        let length = button_side(slot.id)
            .map(|side| format!("    {side}: var({}, 0);\n", length_property(slot.id)))
            .unwrap_or_default();
        let text = !slot.text.is_empty();
        let stack = if text {
            "    position: relative;\n    isolation: isolate;\n"
        } else {
            ""
        };
        let _ = write!(
            out,
            "  {} {{\n    border-image: var({normal});\n    image-rendering: pixelated;\n{length}{stack}  }}\n",
            elements.join(",\n  ")
        );
        painted.push(elements.join(",\n    "));
        if text {
            out.push_str(&backing_rule(slot.id, elements)?);
            backed.extend(elements.iter().map(|e| format!("{e}::before")));
        }
        for state in ORDER.into_iter().filter(|s| slot.has(*s)) {
            let selectors = elements
                .iter()
                .map(|e| {
                    state_selector(e, state).ok_or_else(|| {
                        format!("`{}`: `{e}` cannot take the {} state", slot.id, state.key())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let _ = write!(
                out,
                "  {} {{\n    border-image: var({}, var({normal}));\n  }}\n",
                selectors.join(",\n  "),
                property(slot.id, Some(state))
            );
            painted.push(selectors.join(",\n    "));
        }
    }
    // Last, at each rule's own specificity, so every state is reset too.
    out.push_str("  @media (forced-colors: active) {\n");
    for list in &painted {
        let _ = write!(out, "    {list} {{\n      border-image: none;\n    }}\n");
    }
    if !backed.is_empty() {
        let _ = write!(
            out,
            "    {} {{\n      content: none;\n    }}\n",
            backed.join(",\n    ")
        );
    }
    out.push_str("  }\n}\n");
    Ok(out)
}

/// A serving base the Host owns: ASCII letters, digits and `:/._-`.
fn base_ok(base: &str) -> bool {
    !base.is_empty()
        && base
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":/._-".contains(&b))
}

/// The `border-image` value of an image at `href` under `fit`.
fn value(href: &str, fit: RegionFit) -> String {
    match fit {
        RegionFit::NineSlice(i) => format!(
            "url(\"{href}\") {} {} {} {} fill / {}px {}px {}px {}px / 0 stretch",
            i.top, i.right, i.bottom, i.left, i.top, i.right, i.bottom, i.left
        ),
        RegionFit::Stretch => format!("url(\"{href}\") 0 fill / 0 / 0 stretch"),
        RegionFit::Tile => format!("url(\"{href}\") 0 fill / 0 / 0 repeat"),
    }
}

/// The href of the canonical image `path`: `{base}/{blake3}.{png|gif}`.
fn href(base: &str, path: &str, images: &BTreeMap<String, Vec<u8>>) -> Option<String> {
    let bytes = images.get(path)?;
    let ext = match extension(path).as_deref() {
        Some("gif") => "gif",
        Some("png") => "png",
        _ => return None,
    };
    Some(format!("{base}/{}.{ext}", blake3::hash(bytes).to_hex()))
}

/// The width and height of the canonical image `path`, from its header.
fn dimensions(path: &str, images: &BTreeMap<String, Vec<u8>>) -> Option<(u32, u32)> {
    let kind = match extension(path).as_deref() {
        Some("gif") => ImageKind::Gif,
        Some("png") => ImageKind::Png,
        _ => return None,
    };
    let header = read_header(images.get(path)?, kind, MAX_IMAGE_ALLOC).ok()?;
    Some((header.width, header.height))
}

/// The skin half: `@layer skin { :root { … } }` with each painted region's
/// custom properties, the images addressed under `base` (the Host's
/// serving origin and path). `images` are the canonical images.
///
/// A text slot's backing is written as its token's custom property.
///
/// `None` for a `base` outside its alphabet, a region image `images`
/// lacks, a scrollbar button image whose header does not read, or a text
/// slot whose `textBacking` is missing or no registry colour (never for a
/// validated package).
#[must_use]
pub fn skin_css(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<String> {
    if !base_ok(base) {
        return None;
    }
    let mut decls = String::new();
    for (id, region) in &manifest.regions {
        let Some(slot) = slot_of(id).filter(|s| s.css) else {
            continue;
        };
        let normal = href(base, &region.image, images)?;
        let _ = writeln!(
            decls,
            "    {}: {};",
            property(id, None),
            value(&normal, region.fit)
        );
        if let Some(side) = button_side(id) {
            let (width, height) = dimensions(&region.image, images)?;
            let px = if side == "height" { height } else { width };
            let _ = writeln!(decls, "    {}: {px}px;", length_property(id));
        }
        if !slot.text.is_empty() {
            let token = region.text_backing.as_deref()?;
            let entry = ENTRIES
                .iter()
                .find(|e| e.id == token && matches!(e.ty, TokenType::Color))?;
            let _ = writeln!(decls, "    {}: var({});", backing_property(id), entry.css);
        }
        for (state, path) in &region.states {
            let h = href(base, path, images)?;
            let _ = writeln!(
                decls,
                "    {}: {};",
                property(id, Some(*state)),
                value(&h, region.fit)
            );
        }
    }
    Some(format!("@layer skin {{\n  :root {{\n{decls}  }}\n}}\n"))
}

#[cfg(test)]
mod tests;
