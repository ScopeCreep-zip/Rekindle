//! Every label slot's backing over its own art (plan E7.7.7, the lead's
//! screenshot inspection V2: in Xfire Classic each menu item, community
//! header, `#channel`, "Set topic…" and member group label sat on a hard
//! box over the header art).
//!
//! [`super::title`] decides the title's backing from the titlebar strip's
//! pixels; this is the same rule for every other labelled slot
//! ([`super::slots`], `regions.toml` `labels = [...]`). Where its art fails, the
//! container paints the slot's `textBacking` once behind its text row, so
//! its text never depends on a bitmap it was not measured against (WCAG
//! 2.2 F83). Here the slot's
//! art is measured for the label's pairs (each `RegionText` of the slot,
//! SC 1.4.3: 4.5:1, 3:1 for large text): the pixels under where its
//! labels render ([`super::label_box`]: each label's text line in its
//! container's padding, at every box height the slot takes and every
//! window width, after the image is scaled as `border-image` draws it), of
//! the image, of each state's image and of each frame, tinted as the region
//! says, over a black and a white desktop for its transparent pixels. Art
//! outside the text rows (a bevel, an edge) is never under a label's text,
//! so it never decides the backing. When the worst pixel under the labels
//! meets the floor the slot has no band for that skin; else the skin layer
//! sets it and the container paints the backing once, across its whole
//! strip ([`super::css`]), never a box per label or hugging the text row.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_types::dto::skin::{SkinManifest, TokenValue};

use super::css::{backing_property, band_property};
use super::label_box::label_worst;
use super::title::{TitleGround, TITLE_REGION};
use super::{frames, slot_of, Tokens};
use crate::access::scaled;
use crate::registry::TokenEntry;

/// Each labelled slot's ground (but the titlebar's, [`super::title`]) on
/// the resolved variant `resolved`, by slot id; a slot whose image, frames
/// or colours do not read is left out (its backing stays).
#[must_use]
pub fn label_grounds(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    resolved: &[(&TokenEntry, TokenValue)],
) -> BTreeMap<String, TitleGround> {
    let tokens = Tokens::new(resolved);
    let chrome = tokens.dims.get("size.chrome-height").copied().unwrap_or(0);
    manifest
        .regions
        .iter()
        .filter(|(id, _)| id.as_str() != TITLE_REGION)
        .filter_map(|(id, region)| {
            let slot = slot_of(id).filter(|s| s.labelled && !s.text.is_empty())?;
            let tint = match &region.tint {
                Some(t) => Some(tokens.colours.get(t.as_str()).copied()?),
                None => None,
            };
            let mut worst = f64::INFINITY;
            let mut required = 0u32;
            for t in slot.text {
                let text = tokens.colours.get(t.token).copied()?;
                required = required.max(if t.large { 300 } else { 450 });
                for path in std::iter::once(&region.image).chain(region.states.values()) {
                    for frame in frames(path, images.get(path)?).ok()? {
                        let extents = (slot.width, slot.height);
                        let w = label_worst(id, extents, chrome, &frame, region.fit, tint, text)?;
                        worst = worst.min(w);
                    }
                }
            }
            Some((
                id.clone(),
                TitleGround {
                    worst_x100: scaled(worst, 100.0),
                    required_x100: required,
                },
            ))
        })
        .collect()
}

/// The skin layer setting the band of each labelled slot (but the
/// titlebar's) whose own art does not carry its labels on `resolved`: once
/// per slot, across its whole strip ([`super::css`]). A slot whose art meets
/// its floor gets no band; one whose art does not read gets the band.
#[must_use]
pub fn label_css(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    resolved: &[(&TokenEntry, TokenValue)],
) -> String {
    let mut decls = String::new();
    let grounds = label_grounds(manifest, images, resolved);
    for id in manifest
        .regions
        .keys()
        .filter(|id| id.as_str() != TITLE_REGION)
    {
        if !slot_of(id).is_some_and(|s| s.labelled && !s.text.is_empty()) {
            continue;
        }
        if !grounds.get(id).is_some_and(TitleGround::bare) {
            let _ = writeln!(
                decls,
                "    {}: var({});",
                band_property(id),
                backing_property(id)
            );
        }
    }
    if decls.is_empty() {
        return String::new();
    }
    super::css::scope::skin_layer(&decls)
}

#[cfg(test)]
#[path = "labels_tests.rs"]
mod tests;
