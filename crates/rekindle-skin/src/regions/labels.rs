//! Every label slot's backing over its own art (plan E7.7.7, the lead's
//! screenshot inspection V2: in Xfire Classic each menu item, community
//! header, `#channel`, "Set topic…" and member group label sat on a hard
//! box over the header art).
//!
//! [`super::title`] decides the title's backing from the titlebar strip's
//! pixels; this is the same rule for every other labelled slot
//! ([`super::slots`], `regions.toml` `labels = [...]`). A label paints the
//! slot's `textBacking` behind its content box so its text never depends
//! on a bitmap it was not measured against (WCAG 2.2 F83). Here the slot's
//! art is measured for the label's pairs (each `RegionText` of the slot,
//! SC 1.4.3: 4.5:1, 3:1 for large text): every pixel of the image, of each
//! state's image and of each frame, tinted as the region says, over a black
//! and a white desktop for its transparent pixels. A label can sit over any
//! part of a bar at any size (a nine-slice, stretch or tile only maps
//! output pixels onto source pixels), so every source pixel is a pixel some
//! label may cover: when the worst of them meets the floor the slot's
//! backing is `transparent` for that skin, else it stays.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_types::dto::skin::{SkinManifest, TokenValue};

use super::css::backing_property;
use super::title::{TitleGround, TITLE_REGION};
use super::{frames, slot_of, worst_contrast, Tokens};
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
                        let cols: Vec<u32> = (0..frame.width()).collect();
                        let rows: Vec<u32> = (0..frame.height()).collect();
                        worst = worst.min(worst_contrast(&frame, &cols, &rows, tint, [0; 4], text));
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

/// The skin layer dropping the backing of each labelled slot whose own
/// art carries its labels on `resolved`, else nothing.
#[must_use]
pub fn label_css(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    resolved: &[(&TokenEntry, TokenValue)],
) -> String {
    let mut decls = String::new();
    for (id, g) in label_grounds(manifest, images, resolved) {
        if g.bare() {
            let _ = writeln!(decls, "    {}: transparent;", backing_property(&id));
        }
    }
    if decls.is_empty() {
        return String::new();
    }
    format!("@layer skin {{\n  :root {{\n{decls}  }}\n}}\n")
}

#[cfg(test)]
#[path = "labels_tests.rs"]
mod tests;
