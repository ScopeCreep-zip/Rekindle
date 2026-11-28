//! Status icons against the rules the token look meets (plan E7.7.1
//! "Icons", E7.7.7 S4): a skin's status icon is the fill of the app-owned
//! StatusDot shape (`src/components/status/StatusDot.tsx`), so it must
//! pass what the status colour it replaces passes.
//!
//! * Size: square and one of [`ICON_SIDES`], so the 12 px dot scales it by
//!   a whole or half step.
//! * S2: the fill against the shape's cutout ground (`--sec-status-cutout`)
//!   at 3:1, for every slot whose shape has a cutout (not the offline and
//!   invisible ring), as `access::cutout_pair` does for the colour.
//! * S3: the fill against every buddy-row ground ([`STATUS_ROWS`]) at 3:1
//!   (WCAG 1.4.11 non-text contrast).
//!
//! Both are measured at the worst pixel of the part the shape shows (the
//! inner disc, or the ring's band, away from the anti-aliased edge),
//! each pixel composited over the ground, as F83 measures text over a
//! bitmap: a transparent hole in the fill shows the ground and fails.
//! Every frame of an animated icon is measured; the flash rules already ran
//! on every icon in step 4 ([`crate::images`], drawn unbounded).

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule};

use super::pairs::{Floor, STATUS_ROWS};
use super::{cutout_ground, describe, scaled, Variant};
use crate::colour;
use crate::registry::{Base, IconSlot, TokenEntry};
use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::TokenValue;

/// The sides a status icon may have, in px.
pub const ICON_SIDES: [u32; 5] = [12, 16, 24, 32, 48];

/// Which part of the 12-unit StatusDot box shows the fill: the disc of
/// radius 4.5, or the offline ring's band (radius 4, stroke 1.5).
fn shows(slot: IconSlot, x: u32, y: u32, side: u32) -> bool {
    let k = f64::from(side) / 12.0;
    let dx = (f64::from(x) + 0.5) / k - 6.0;
    let dy = (f64::from(y) + 0.5) / k - 6.0;
    let d = dx.hypot(dy);
    match slot {
        IconSlot::Offline | IconSlot::Invisible => (3.4..=4.6).contains(&d),
        _ => d <= 3.6,
    }
}

/// The worst contrast of the shown pixels of every frame over `ground`.
fn worst(frames: &[RgbaImage], slot: IconSlot, ground: [u8; 3]) -> f64 {
    let mut worst = f64::INFINITY;
    for frame in frames {
        for (x, y, p) in frame.enumerate_pixels() {
            if shows(slot, x, y, frame.width()) {
                let seen = colour::over(p.0, ground);
                worst = worst.min(colour::contrast(seen, ground));
            }
        }
    }
    worst
}

/// Checks every status icon of `manifest` on the variant `resolved` of
/// `base`.
///
/// # Errors
/// An icon that is not square at one of [`ICON_SIDES`], or whose fill
/// falls below 3:1 against the cutout or a buddy-row ground.
pub fn validate(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    base: Base,
    resolved: &[(&TokenEntry, TokenValue)],
) -> Result<(), SkinError> {
    let v = Variant::new(resolved);
    for (id, path) in &manifest.icons {
        let Some(slot) = IconSlot::of(id) else {
            continue;
        };
        let Some(bytes) = images.get(path) else {
            continue;
        };
        let frames = crate::regions::frames(path, bytes)?;
        let refuse = |rule| SkinError::new(path.as_str(), rule);
        if let Some(f) = frames
            .iter()
            .find(|f| f.width() != f.height() || !ICON_SIDES.contains(&f.width()))
        {
            return Err(refuse(SkinRule::IconSize {
                icon: id.clone(),
                width: f.width(),
                height: f.height(),
            }));
        }
        let required = Floor::Graphic.x100();
        let mut grounds: Vec<(&str, [u8; 3], String)> = STATUS_ROWS
            .iter()
            .map(|g| ("S3", v.surface(g), describe(&v, g)))
            .collect();
        if !matches!(slot, IconSlot::Offline | IconSlot::Invisible) {
            grounds.push(("S2", cutout_ground(base), "the StatusDot cutout".to_owned()));
        }
        for (rule, ground, name) in grounds {
            let ratio = worst(&frames, slot, ground);
            if ratio * 100.0 < f64::from(required) {
                return Err(refuse(SkinRule::Contrast {
                    rule: rule.to_owned(),
                    token: format!("icons.{id}"),
                    ground: name,
                    measured_x100: scaled(ratio, 100.0),
                    required_x100: u32::from(required),
                }));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "icons_tests.rs"]
mod tests;
