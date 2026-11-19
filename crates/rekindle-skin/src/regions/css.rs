//! Region CSS (plan E7.7.1 "Regions", validator step 7): two halves, so no
//! skin string ever names a selector.
//!
//! * [`app_css`], generated into `src/styles/generated/regions.css` by
//!   `cargo xtask gen-skin-registry` from the `[regions]` table of
//!   `src/styles/regions.toml`: in `@layer components`, after every app
//!   rule, each painted slot's elements and their state selectors read the
//!   slot's custom property into `border-image`. A state without its own
//!   image shows the normal one; with no skin region the property is unset
//!   and `border-image` computes to `none`. [`app_a11y_css`], generated
//!   beside it into `regions-a11y.css`, holds the same rules under the
//!   user's settings (below).
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
//! * The user's settings follow in `@layer a11y` (plan E7.7.2 "Apply";
//!   `src/styles/global.css:16` declares `… components, utilities, a11y,
//!   sec`, so they beat every app and skin rule and only `@layer sec` sits
//!   above them). Under `prefers-reduced-motion: reduce` each region rule
//!   reads its image's still ([`still_property`]: the first frame of an
//!   animated GIF, [`super::still`], which [`skin_css`] sets only for a
//!   GIF that moves), else the image itself.
//! * Under `forced-colors: active` every region rule's `border-image` is
//!   `none`, after the reduced-motion rules so it wins over them too: a
//!   forced palette replaces the app's colours, and a skin bitmap left in
//!   place would draw the skin's colours over it. CSS Color Adjustment 1
//!   §3.1 (<https://www.w3.org/TR/css-color-adjust-1/#forced-colors-properties>)
//!   forces the colour properties and computes `background-image` to `none`
//!   "unless the original value contains a url() function"; it does not
//!   name `border-image`, the only property a region image is drawn with
//!   here. The one `background-image` these rules write, a text slot's
//!   backing gradient on its `::before`, goes with the box (`content:
//!   none`).
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
//! * The backing is two layers of that colour, so a skin's nine-slice
//!   image shows its edge slices and not only the ring of its border: one
//!   on the padding box inset by [`slice_property`] on each side (the
//!   image's slice widths, which [`skin_css`] sets; 0 with no skin or a
//!   `stretch`/`tile` image, so the backing is then the whole padding box),
//!   and one on the content box (`padding: inherit`), so the label is over
//!   the backing even in a variant whose padding is narrower than a slice
//!   (`super::validate` samples exactly that band). `border-radius:
//!   inherit` rounds the padding box; the inset layer is inside its corner
//!   arcs once a slice reaches the radius, and the image's own corners are
//!   the skin's.
//! * A labelled container slot (a bar, a header, the selected row: plan
//!   E7.7.7) paints its bitmap on the container and the backing on each
//!   label the `[regions]` entry names (`labels = [...]`), as the label's
//!   own `background-image` (a one-colour gradient of
//!   [`backing_property`]): a child's background paints over its
//!   ancestor's border image and under its own text, so the bitmap shows
//!   around the labels and never under their text. Unset, the gradient is
//!   invalid at computed-value time and `background-image` is `none`, so
//!   without a skin the labels look as the app draws them. Under
//!   `forced-colors: active` the labels' `background-image` is `none` too.
//! * A sized slot's element (the status arrow) is as wide and tall as its
//!   image ([`size_property`]) and 0 × 0 without a skin.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_image::{read_header, ImageKind};
use rekindle_types::dto::skin::{RegionFit, RegionState, SkinManifest};

use super::slots::{slot_of, SLOTS};
use super::still;
use crate::images::extension;
use crate::limits::MAX_IMAGE_ALLOC;
use crate::registry::{TokenType, ENTRIES};

/// One `[regions]` entry of `src/styles/regions.toml`: the elements a
/// slot paints and, for a labelled slot, the children carrying its text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionMap {
    pub elements: Vec<String>,
    pub labels: Vec<String>,
}

impl RegionMap {
    /// An entry painting `elements`, with no labels.
    #[must_use]
    pub fn elements(elements: &[&str]) -> Self {
        Self {
            elements: elements.iter().map(|e| (*e).to_owned()).collect(),
            labels: Vec::new(),
        }
    }

    /// An entry painting `elements` with its text on `labels`.
    #[must_use]
    pub fn labelled(elements: &[&str], labels: &[&str]) -> Self {
        Self {
            labels: labels.iter().map(|e| (*e).to_owned()).collect(),
            ..Self::elements(elements)
        }
    }
}

/// The custom property holding a sized slot's `side` (`width` or
/// `height`): its image's, set by [`skin_css`].
#[must_use]
pub fn size_property(slot: &str, side: &str) -> String {
    format!("{}-{side}", property(slot, None))
}

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

/// The custom property holding a text slot's nine-slice width on `side`
/// (`top`, `right`, `bottom` or `left`).
#[must_use]
pub fn slice_property(slot: &str, side: &str) -> String {
    format!("{}-slice-{side}", property(slot, None))
}

/// The sides of a nine-slice, in CSS order.
const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// The `::before` of a text slot's `elements` that paints the backing
/// under the label: the image's centre and the label's content box.
fn backing_rule(id: &str, elements: &[String]) -> Result<String, String> {
    if let Some(e) = elements.iter().find(|e| e.contains("::")) {
        return Err(format!(
            "`{id}`: `{e}` carries text, so it must be an element, not a pseudo-element"
        ));
    }
    let before: Vec<String> = elements.iter().map(|e| format!("{e}::before")).collect();
    let [t, r, b, l] = SIDES.map(|side| format!("var({}, 0px)", slice_property(id, side)));
    let fill = format!("linear-gradient(var({0}), var({0}))", backing_property(id));
    Ok(format!(
        "  {} {{\n    content: \"\";\n    position: absolute;\n    inset: 0;\n    z-index: -1;\n    padding: inherit;\n    border-radius: inherit;\n    background:\n      {fill} {l} {t} / calc(100% - {l} - {r}) calc(100% - {t} - {b}) no-repeat padding-box,\n      {fill} no-repeat content-box;\n    pointer-events: none;\n  }}\n",
        before.join(",\n  ")
    ))
}

/// The custom property holding the still of `slot`'s (or its `state`'s)
/// image: set by [`skin_css`] only when that image is an animated GIF.
#[must_use]
pub fn still_property(slot: &str, state: Option<RegionState>) -> String {
    format!("{}--still", property(slot, state))
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
        RegionState::Focused | RegionState::Selected if pseudo => return None,
        RegionState::Selected => format!("{element}[aria-selected=\"true\"]"),
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
const ORDER: [RegionState; 6] = [
    RegionState::WindowInactive,
    RegionState::Hover,
    RegionState::Focused,
    RegionState::Selected,
    RegionState::Down,
    RegionState::Disabled,
];

/// `src/styles/generated/regions.css` from the region map's `[regions]`
/// (slot id → the app elements it paints), with `header` as its comment:
/// the region rules, in `@layer components`.
///
/// # Errors
/// A map entry naming no painted slot, a painted slot the map lacks or
/// maps to no element, or a state a pseudo-element cannot take.
pub fn app_css(header: &str, map: &BTreeMap<String, RegionMap>) -> Result<String, String> {
    let (components, _) = rules(map)?;
    Ok(format!(
        "/* {header}\n *\n * Region slots (`rekindle-skin::regions`) on the elements\n * `src/styles/regions.toml` `[regions]` names; a skin sets the custom\n * properties in `@layer skin`. The user's settings are in\n * `regions-a11y.css`. */\n@layer components {{\n{components}}}\n"
    ))
}

/// `src/styles/generated/regions-a11y.css` from the same map: the same
/// rules under the user's settings, in `@layer a11y` (plan E7.7.2
/// "Apply"): the reduced-motion swap to each image's still, then the
/// forced-colors reset.
///
/// # Errors
/// What [`app_css`] returns.
pub fn app_a11y_css(header: &str, map: &BTreeMap<String, RegionMap>) -> Result<String, String> {
    let (_, a11y) = rules(map)?;
    Ok(format!(
        "/* {header}\n *\n * The region rules of `regions.css` under the user's settings, in\n * `@layer a11y`, which `global.css` orders above `components` and\n * `utilities` and below `sec` only. */\n@layer a11y {{\n{a11y}}}\n"
    ))
}

/// The bodies of the region rules' `@layer components` and `@layer a11y`.
fn rules(map: &BTreeMap<String, RegionMap>) -> Result<(String, String), String> {
    for id in map.keys() {
        if !slot_of(id).is_some_and(|s| s.css) {
            return Err(format!("[regions] `{id}`: not a painted region slot"));
        }
    }
    // Every rule's selector list, for the forced-colors reset, and every
    // backing `::before`.
    let mut painted: Vec<String> = Vec::new();
    let mut backed: Vec<String> = Vec::new();
    // Every label of a labelled slot.
    let mut labels: Vec<String> = Vec::new();
    // Every rule again, reading the still under reduced motion.
    let mut motion = String::new();
    let mut out = String::new();
    for slot in SLOTS.iter().filter(|s| s.css) {
        let entry = map
            .get(slot.id)
            .filter(|e| !e.elements.is_empty())
            .ok_or_else(|| format!("[regions] has no elements for `{}`", slot.id))?;
        let elements = &entry.elements;
        match (slot.labelled, entry.labels.is_empty()) {
            (true, true) => {
                return Err(format!(
                    "[regions] `{}` carries text on its children: it needs `labels = [...]`",
                    slot.id
                ))
            }
            (false, false) => {
                return Err(format!(
                    "[regions] `{}` is not a labelled slot: it takes no `labels`",
                    slot.id
                ))
            }
            _ => {}
        }
        if let Some(l) = entry.labels.iter().find(|l| l.contains("::")) {
            return Err(format!(
                "`{}`: label `{l}` carries text, so it must be an element",
                slot.id
            ));
        }
        let normal = property(slot.id, None);
        let length = button_side(slot.id)
            .map(|side| format!("    {side}: var({}, 0);\n", length_property(slot.id)))
            .unwrap_or_default();
        let size = if slot.sized {
            format!(
                "    width: var({}, 0);\n    height: var({}, 0);\n",
                size_property(slot.id, "width"),
                size_property(slot.id, "height")
            )
        } else {
            String::new()
        };
        let text = !slot.text.is_empty() && !slot.labelled;
        let stack = if text {
            "    position: relative;\n    isolation: isolate;\n"
        } else {
            ""
        };
        let _ = write!(
            out,
            "  {} {{\n    border-image: var({normal});\n    image-rendering: pixelated;\n{length}{size}{stack}  }}\n",
            elements.join(",\n  ")
        );
        painted.push(elements.join(",\n    "));
        let normal_still = format!("var({}, var({normal}))", still_property(slot.id, None));
        let _ = write!(
            motion,
            "    {} {{\n      border-image: {normal_still};\n    }}\n",
            elements.join(",\n    ")
        );
        if slot.labelled {
            let _ = write!(
                out,
                "  {} {{\n    background-image: linear-gradient(var({b}), var({b}));\n  }}\n",
                entry.labels.join(",\n  "),
                b = backing_property(slot.id)
            );
            labels.extend(entry.labels.iter().cloned());
        }
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
            let _ = write!(
                motion,
                "    {} {{\n      border-image: var({}, var({}, {normal_still}));\n    }}\n",
                selectors.join(",\n    "),
                still_property(slot.id, Some(state)),
                property(slot.id, Some(state))
            );
        }
    }
    let mut a11y = format!("  @media (prefers-reduced-motion: reduce) {{\n{motion}  }}\n");
    // Last, at each rule's own specificity, so every state is reset too.
    a11y.push_str("  @media (forced-colors: active) {\n");
    for list in &painted {
        let _ = write!(a11y, "    {list} {{\n      border-image: none;\n    }}\n");
    }
    if !backed.is_empty() {
        let _ = write!(
            a11y,
            "    {} {{\n      content: none;\n    }}\n",
            backed.join(",\n    ")
        );
    }
    if !labels.is_empty() {
        let _ = write!(
            a11y,
            "    {} {{\n      background-image: none;\n    }}\n",
            labels.join(",\n    ")
        );
    }
    a11y.push_str("  }\n");
    Ok((out, a11y))
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
/// A text slot's backing is written as its token's custom property, and
/// its image's slice widths (0 for `stretch` and `tile`) as
/// [`slice_property`], where its backing's centre layer begins. An
/// animated GIF (normal or state) also gets its still under
/// [`still_property`], addressed by the still's own BLAKE3 as a `.png`
/// ([`super::still`]), which `@layer a11y` reads under reduced motion.
///
/// `None` for a `base` outside its alphabet, a region image `images`
/// lacks or whose still does not decode, a scrollbar button image whose
/// header does not read, or a text
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
    let stills = still::stills(manifest, images).ok()?;
    // The still's href and value, for an animated image `path`.
    let still_decl = |prop: String, path: &str, fit: RegionFit| {
        stills.get(path).map(|png| {
            let h = format!("{base}/{}.png", blake3::hash(png).to_hex());
            format!("    {prop}: {};\n", value(&h, fit))
        })
    };
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
        if let Some(d) = still_decl(still_property(id, None), &region.image, region.fit) {
            decls.push_str(&d);
        }
        if slot.sized {
            let (width, height) = dimensions(&region.image, images)?;
            let _ = writeln!(decls, "    {}: {width}px;", size_property(id, "width"));
            let _ = writeln!(decls, "    {}: {height}px;", size_property(id, "height"));
        }
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
            // A labelled slot backs its labels in full: no slice widths.
            let widths = match region.fit {
                _ if slot.labelled => None,
                RegionFit::NineSlice(i) => Some([i.top, i.right, i.bottom, i.left]),
                RegionFit::Stretch | RegionFit::Tile => Some([0; 4]),
            };
            for (side, px) in widths.into_iter().flat_map(|w| SIDES.into_iter().zip(w)) {
                let _ = writeln!(decls, "    {}: {px}px;", slice_property(id, side));
            }
        }
        for (state, path) in &region.states {
            let h = href(base, path, images)?;
            let _ = writeln!(
                decls,
                "    {}: {};",
                property(id, Some(*state)),
                value(&h, region.fit)
            );
            if let Some(d) = still_decl(still_property(id, Some(*state)), path, region.fit) {
                decls.push_str(&d);
            }
        }
    }
    Some(format!("@layer skin {{\n  :root {{\n{decls}  }}\n}}\n"))
}

#[cfg(test)]
mod tests;
