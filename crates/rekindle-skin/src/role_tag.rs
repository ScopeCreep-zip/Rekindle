//! A role tag's readable text colour (plan E7.3, WCAG 2.2 SC 1.4.3).
//!
//! A role tag (`src/components/community/RoleTag.tsx`) is the role's
//! name in the role's user-chosen colour on a tint of that colour,
//! `color-mix(in srgb, <colour> 25%, transparent)`, laid on the grounds
//! the region map names for it (`src/styles/regions.toml`,
//! `.role-tag-default`: `bg.dark`, `bg.panel`, `bg.primary`). A chosen
//! colour need not read on those grounds, so the Host sends, per skin
//! base, the colour the name is drawn in: the role's own where it reaches
//! 4.5:1 on the tag over every such ground of that base's token skin and
//! of the skin this device shows ([`SkinGrounds`], a built-in or an
//! imported one), else the smallest Oklab-lightness move of it that does
//! ([`walk::lightness_walk`], the editor's walk), towards white on a dark
//! base and black on a light one. The tint stays the role's own colour, so
//! the tag still shows it.
//!
//! The colour is worked out per device: every window of a device shows the
//! one skin the Host serves, and the Host re-sends the roles whose colours
//! change when that skin changes (`rekindle-node` `host/skin/shown.rs`,
//! from no skin too), so the grounds of skins the device does not show are
//! not checked. Checking every built-in's (plan E7.2 slice 3, before the
//! Host knew the shown skin) pushed every tag towards white once the packs'
//! own grounds were imported (XGlow's buddy list is #494949, plan E7.7.7).
//! The token skin stays in the set so a tag drawn before the first
//! resolution reads on the default skin.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use rekindle_types::dto::community::RoleTagColours;
use rekindle_types::dto::skin::{SkinBase, SkinContrast, SkinError, TokenValue};

use crate::colour;
use crate::import::walk;
use crate::registry::{resolve, Base, TokenEntry, ENTRIES};
use crate::{manifest, package, serve};

/// The grounds a role tag is laid on (`regions.toml`, `.role-tag-default`).
pub const GROUNDS: [&str; 3] = ["bg.dark", "bg.panel", "bg.primary"];

/// The tag's tint of the role colour, per mille (`RoleTag.tsx`'s 25%).
pub const TINT: u16 = 250;

/// SC 1.4.3's ratio for the tag's 9 px text.
const TEXT_RATIO: f64 = 4.5;

fn rgb_of(resolved: &[(&TokenEntry, TokenValue)], id: &str) -> Option<[u8; 3]> {
    resolved.iter().find_map(|(e, v)| match v {
        TokenValue::Color { r, g, b, .. } if e.id == id => Some([*r, *g, *b]),
        _ => None,
    })
}

fn push_grounds(out: &mut Vec<[u8; 3]>, resolved: &[(&TokenEntry, TokenValue)]) {
    out.extend(GROUNDS.iter().filter_map(|id| rgb_of(resolved, id)));
}

/// The role-tag grounds of the token skin on `base`.
fn collect(base: SkinBase) -> Vec<[u8; 3]> {
    let mut out = Vec::new();
    push_grounds(
        &mut out,
        &resolve(
            ENTRIES,
            Base::of(base, SkinContrast::Standard),
            &BTreeMap::new(),
        ),
    );
    out
}

/// The role-tag grounds of one skin: the base its applied variant declares
/// and that variant's `.role-tag-default` grounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinGrounds {
    pub base: SkinBase,
    pub grounds: Vec<[u8; 3]>,
}

impl SkinGrounds {
    /// The grounds of the canonical package `bytes` as it is applied.
    ///
    /// # Errors
    /// A package that does not read, parse or resolve.
    pub fn of_package(bytes: &[u8]) -> Result<Self, SkinError> {
        let p = package::read(bytes)?;
        let m = manifest::parse(&p)?;
        let (v, resolved) = serve::active_resolved(&p, &m)?;
        let mut grounds = Vec::new();
        push_grounds(&mut grounds, &resolved);
        Ok(Self {
            base: v.base,
            grounds,
        })
    }
}

/// The token skin's role-tag grounds on `base`, worked out once per process.
#[must_use]
pub fn grounds(base: SkinBase) -> &'static [[u8; 3]] {
    static DARK: OnceLock<Vec<[u8; 3]>> = OnceLock::new();
    static LIGHT: OnceLock<Vec<[u8; 3]>> = OnceLock::new();
    match base {
        SkinBase::Dark => DARK.get_or_init(|| collect(SkinBase::Dark)),
        SkinBase::Light => LIGHT.get_or_init(|| collect(SkinBase::Light)),
    }
}

/// The tag's opaque ground: the role colour's tint over `ground`.
fn tag(rgb: [u8; 3], ground: [u8; 3]) -> [u8; 3] {
    let [r, g, b, _] = colour::mix(
        [rgb[0], rgb[1], rgb[2], 255],
        [ground[0], ground[1], ground[2], 255],
        TINT,
    );
    [r, g, b]
}

fn unpack(c: u32) -> [u8; 3] {
    let [_, r, g, b] = c.to_be_bytes();
    [r, g, b]
}

fn pack([r, g, b]: [u8; 3]) -> u32 {
    u32::from_be_bytes([0, r, g, b])
}

/// The text colour of a tag of `colour` (`0xRRGGBB`) on `base`, over the
/// token skin's grounds and the shown skin's (`shown`) when it is on
/// `base`.
#[must_use]
pub fn readable(colour: u32, base: SkinBase, shown: Option<&SkinGrounds>) -> u32 {
    let rgb = unpack(colour);
    let own = shown
        .filter(|s| s.base == base)
        .map(|s| s.grounds.as_slice());
    let tags: Vec<[u8; 3]> = grounds(base)
        .iter()
        .chain(own.unwrap_or_default())
        .map(|g| tag(rgb, *g))
        .collect();
    let passes = |c: [u8; 3]| tags.iter().all(|t| colour::contrast(c, *t) >= TEXT_RATIO);
    if passes(rgb) {
        return colour & 0x00ff_ffff;
    }
    let up = base == SkinBase::Dark;
    let walked = walk::lightness_walk(rgb, up, |c| Ok::<_, std::convert::Infallible>(passes(c)))
        .ok()
        .flatten();
    // The walk's end keeps the hue; past it only the achromatic end is left.
    pack(walked.unwrap_or(if up { [0xff; 3] } else { [0; 3] }))
}

/// Both bases' text colours of a tag of `colour`, given the skin this
/// device shows (`None` where no device is: the fixtures).
#[must_use]
pub fn colours(colour: u32, shown: Option<&SkinGrounds>) -> RoleTagColours {
    RoleTagColours {
        dark: readable(colour, SkinBase::Dark, shown),
        light: readable(colour, SkinBase::Light, shown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_base_has_the_token_skin_grounds() {
        assert_eq!(grounds(SkinBase::Dark).len(), GROUNDS.len());
        assert_eq!(grounds(SkinBase::Light).len(), GROUNDS.len());
    }

    #[test]
    fn a_colour_that_reads_is_kept() {
        assert_eq!(readable(0x00ff_ffff, SkinBase::Dark, None), 0x00ff_ffff);
        assert_eq!(readable(0x0000_0000, SkinBase::Light, None), 0);
    }

    #[test]
    fn the_shown_skin_grounds_join_its_base_only() {
        // A shown dark skin with a lighter ground (#404040; the token
        // skin's are #0a-#1e): #c0c0c0 reads on the token skin's grounds,
        // and a pack the device does not show (XGlow's #494949 buddy list)
        // does not move it, but it is 3.5:1 on its tag over #404040, so it
        // moves; the light base's colour is not touched by a dark skin.
        let colour = 0x00c0_c0c0;
        assert_eq!(readable(colour, SkinBase::Dark, None), colour);
        let shown = SkinGrounds {
            base: SkinBase::Dark,
            grounds: vec![[0x40; 3]],
        };
        let out = readable(colour, SkinBase::Dark, Some(&shown));
        assert_ne!(out, colour);
        let ground = tag(unpack(colour), [0x40; 3]);
        assert!(colour::contrast(unpack(out), ground) >= TEXT_RATIO);
        assert_eq!(
            readable(colour, SkinBase::Light, Some(&shown)),
            readable(colour, SkinBase::Light, None)
        );
    }

    #[test]
    fn a_colour_that_does_not_read_moves_until_it_does_on_every_ground() {
        // #3498db on white is 3.2:1 (the axe finding on the light base).
        for (base, colour) in [
            (SkinBase::Light, 0x0034_98db),
            (SkinBase::Dark, 0x0020_2080),
        ] {
            let out = readable(colour, base, None);
            assert_ne!(out, colour, "{base:?}");
            for g in grounds(base) {
                let ground = tag(unpack(colour), *g);
                assert!(
                    colour::contrast(unpack(out), ground) >= TEXT_RATIO,
                    "{base:?} {g:?}"
                );
            }
        }
    }
}
