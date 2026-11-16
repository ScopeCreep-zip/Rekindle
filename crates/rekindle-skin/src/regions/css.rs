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
//! image's transparent pixels. Painted slots are leaf controls, so their
//! `image-rendering: pixelated` (the F83 sampling's assumption) reaches no
//! other image.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_types::dto::skin::{RegionFit, RegionState, SkinManifest};

use super::slots::{slot_of, SLOTS};
use crate::images::extension;

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
    let mut out = format!(
        "/* {header}\n *\n * Region slots (`rekindle-skin::regions`) on the elements\n * `src/styles/regions.toml` `[regions]` names; a skin sets the custom\n * properties in `@layer skin`. */\n@layer components {{\n"
    );
    for slot in SLOTS.iter().filter(|s| s.css) {
        let elements = map
            .get(slot.id)
            .filter(|e| !e.is_empty())
            .ok_or_else(|| format!("[regions] has no elements for `{}`", slot.id))?;
        let normal = property(slot.id, None);
        let _ = write!(
            out,
            "  {} {{\n    border-image: var({normal});\n    image-rendering: pixelated;\n  }}\n",
            elements.join(",\n  ")
        );
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
        }
    }
    out.push_str("}\n");
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

/// The skin half: `@layer skin { :root { … } }` with each painted region's
/// custom properties, the images addressed under `base` (the Host's
/// serving origin and path). `images` are the canonical images.
///
/// `None` for a `base` outside its alphabet or a region image `images`
/// lacks (never for a validated package).
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
        if !slot_of(id).is_some_and(|s| s.css) {
            continue;
        }
        let normal = href(base, &region.image, images)?;
        let _ = writeln!(
            decls,
            "    {}: {};",
            property(id, None),
            value(&normal, region.fit)
        );
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
