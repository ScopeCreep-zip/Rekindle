//! A role tag's readable text colour (plan E7.3, WCAG 2.2 SC 1.4.3).
//!
//! A role tag (`src/components/community/RoleTag.tsx`) is the role's
//! name in the role's user-chosen colour on a tint of that colour,
//! `color-mix(in srgb, <colour> 25%, transparent)`, laid on the grounds
//! the region map names for it (`src/styles/regions.toml`,
//! `.role-tag-default`: `bg.dark`, `bg.panel`, `bg.primary`). A chosen
//! colour need not read on those grounds, so the Host sends, per skin
//! base, the colour the name is drawn in: the role's own where it reaches
//! 4.5:1 on the tag over every such ground of every built-in skin on that
//! base (the token skin and each pack's applied variant), else the
//! smallest Oklab-lightness move of it that does ([`walk::lightness_walk`],
//! the editor's walk), towards white on a dark base and black on a light
//! one. The tint stays the role's own colour, so the tag still shows it.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use rekindle_types::dto::community::RoleTagColours;
use rekindle_types::dto::skin::{SkinBase, SkinContrast, TokenValue};

use crate::builtin::PACKS;
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

/// Every role-tag ground of every built-in skin applied on `base`.
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
    for pack in &PACKS {
        // The embedded packs are canonical (`builtin::tests::
        // every_pack_is_canonical_and_the_default_is_one`).
        let p = package::read(pack.bytes).expect("a built-in pack reads");
        let m = manifest::parse(&p).expect("a built-in manifest parses");
        let (v, resolved) = serve::active_resolved(&p, &m).expect("a built-in variant resolves");
        if v.base == base {
            push_grounds(&mut out, &resolved);
        }
    }
    out
}

/// The role-tag grounds of `base`, worked out once per process.
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

/// The text colour of a tag of `colour` (`0xRRGGBB`) on `base`.
#[must_use]
pub fn readable(colour: u32, base: SkinBase) -> u32 {
    let rgb = unpack(colour);
    let tags: Vec<[u8; 3]> = grounds(base).iter().map(|g| tag(rgb, *g)).collect();
    let passes = |c: [u8; 3]| tags.iter().all(|t| colour::contrast(c, *t) >= TEXT_RATIO);
    if passes(rgb) {
        return colour & 0x00ff_ffff;
    }
    let up = base == SkinBase::Dark;
    let walked = walk::lightness_walk(rgb, up, |c| Ok(passes(c)))
        .ok()
        .flatten();
    // The walk's end keeps the hue; past it only the achromatic end is left.
    pack(walked.unwrap_or(if up { [0xff; 3] } else { [0; 3] }))
}

/// Both bases' text colours of a tag of `colour`.
#[must_use]
pub fn colours(colour: u32) -> RoleTagColours {
    RoleTagColours {
        dark: readable(colour, SkinBase::Dark),
        light: readable(colour, SkinBase::Light),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_base_has_grounds_from_the_token_skin_and_the_packs() {
        // The token skin alone gives three per base.
        assert!(grounds(SkinBase::Dark).len() > GROUNDS.len());
        assert!(grounds(SkinBase::Light).len() >= GROUNDS.len());
    }

    #[test]
    fn a_colour_that_reads_is_kept() {
        assert_eq!(readable(0x00ff_ffff, SkinBase::Dark), 0x00ff_ffff);
        assert_eq!(readable(0x0000_0000, SkinBase::Light), 0);
    }

    #[test]
    fn a_colour_that_does_not_read_moves_until_it_does_on_every_ground() {
        // #3498db on white is 3.2:1 (the axe finding on the light base).
        for (base, colour) in [
            (SkinBase::Light, 0x0034_98db),
            (SkinBase::Dark, 0x0020_2080),
        ] {
            let out = readable(colour, base);
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
