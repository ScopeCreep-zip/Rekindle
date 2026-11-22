//! The built-in packs (plan E7.7.2 "Built-in packs").
//!
//! Each lives in `crates/rekindle-skin/builtin/<id>.rkskin`, written by
//! `cargo xtask import-skin … --builtin <id>` (E7.7.3) beside its `NOTICE`,
//! and is embedded here with `include_bytes!`. Embedding is not trust: a
//! built-in passes the same helper validation as any import, on its first
//! use in each process ([`validated`]). A committed pack is the pipeline's
//! fixed point, so the helper's canonical package must be the pack itself:
//! an answer naming other bytes is refused and never kept, which means no
//! helper, a test's stand-in included, can make a built-in serve anything
//! but its own committed bytes for any later caller. Its address is
//! therefore known without the helper ([`by_address`]).
//!
//! [`DEFAULT`] is Symbiosis, Xfire's stock skin, now that E7.7.3 imported
//! it (`docs/architecture/ui-skin.md` "Reference assets"); `builtin:default`
//! is the registry's token skin, which has no package.

use std::sync::OnceLock;

use crate::helper::{self, HelperError, HelperOpts};
use crate::output::Canonical;

/// The prefix of a built-in's skin id (`app_settings.skin_id`'s `GLOB
/// 'builtin:*'`).
pub const PREFIX: &str = "builtin:";

/// The default skin: the last step of the resolution (the identity's
/// choice, then the device login skin, then this).
pub const DEFAULT: &str = "builtin:symbiosis";

/// One embedded pack.
#[derive(Debug)]
pub struct Builtin {
    /// The id after [`PREFIX`].
    pub id: &'static str,
    /// The pack as committed.
    pub bytes: &'static [u8],
}

/// Every built-in pack, by id: Symbiosis first (the default), then Xfire's
/// other own skins (plan E7.7.3, "Official Xfire skins"): Classic v3 (the
/// id `classic` is the sound bank) and Combat; then the fan skins (plan
/// E7.7.3d, owner 2026-11-17) whose files import, whose authors stated no
/// restriction, and whose own files name their authors (`Info.xml`
/// `<Author>`, or `version.ini` comments for Alloy, Simple X and XGlow),
/// each credited in its manifest and `NOTICE`, and each carrying its art
/// (frame, title buttons, scrollbar; plan E7.7.3e): a fan skin imported
/// colour-only would misrepresent its author's work. Skins carrying game
/// art and names (xBIOSHOCK, Guild Wars UI) are treated like the partner
/// skins and not shipped. An author who asks has their skin removed in the
/// next release (`docs/user/skin-credits.md`).
pub const PACKS: [Builtin; 11] = [
    Builtin {
        id: "symbiosis",
        bytes: include_bytes!("../builtin/symbiosis.rkskin"),
    },
    Builtin {
        id: "xfire-classic",
        bytes: include_bytes!("../builtin/xfire-classic.rkskin"),
    },
    Builtin {
        id: "combat",
        bytes: include_bytes!("../builtin/combat.rkskin"),
    },
    Builtin {
        id: "onyx",
        bytes: include_bytes!("../builtin/onyx.rkskin"),
    },
    Builtin {
        id: "spacecowboy",
        bytes: include_bytes!("../builtin/spacecowboy.rkskin"),
    },
    Builtin {
        id: "stealthx",
        bytes: include_bytes!("../builtin/stealthx.rkskin"),
    },
    Builtin {
        id: "total-chaos",
        bytes: include_bytes!("../builtin/total-chaos.rkskin"),
    },
    Builtin {
        id: "xslimfire",
        bytes: include_bytes!("../builtin/xslimfire.rkskin"),
    },
    Builtin {
        id: "alloy",
        bytes: include_bytes!("../builtin/alloy.rkskin"),
    },
    Builtin {
        id: "simple-x",
        bytes: include_bytes!("../builtin/simple-x.rkskin"),
    },
    Builtin {
        id: "xglow",
        bytes: include_bytes!("../builtin/xglow.rkskin"),
    },
];

/// Each pack's helper-validated canonical package, once per process.
static VALIDATED: [OnceLock<Canonical>; PACKS.len()] = [const { OnceLock::new() }; PACKS.len()];

/// Each pack's address: BLAKE3 of its committed bytes, which is the
/// address of the only canonical package [`validated`] accepts for it.
static ADDRESSES: OnceLock<[String; PACKS.len()]> = OnceLock::new();

/// The pack a `builtin:<id>` skin id names, and its index in [`PACKS`].
#[must_use]
pub fn find(skin_id: &str) -> Option<(usize, &'static Builtin)> {
    let id = skin_id.strip_prefix(PREFIX)?;
    PACKS.iter().enumerate().find(|(_, b)| b.id == id)
}

/// The built-in whose canonical package has `address` (a `skin://` path's
/// first segment), and its index in [`PACKS`]: looked up by address, so
/// serving one file validates one pack, not every pack.
#[must_use]
pub fn by_address(address: &str) -> Option<(usize, &'static Builtin)> {
    let addresses = ADDRESSES
        .get_or_init(|| std::array::from_fn(|i| blake3::hash(PACKS[i].bytes).to_hex().to_string()));
    let i = addresses.iter().position(|a| a == address)?;
    Some((i, &PACKS[i]))
}

/// The canonical package of the built-in `skin_id`: validated by the
/// helper at `opts` on first use in this process, the result kept for
/// every later call. A failure is not kept, so a helper that did not
/// answer is tried again on the next use; nor is an answer that is not
/// the committed pack ([`HelperError::NotTheBuiltin`]), so one host's
/// helper cannot define the built-ins another host serves.
///
/// `Ok(None)` when `skin_id` names no built-in pack.
///
/// # Errors
/// What [`helper::validate`] returns.
pub fn validated(skin_id: &str, opts: &HelperOpts) -> Result<Option<Canonical>, HelperError> {
    let Some((i, pack)) = find(skin_id) else {
        return Ok(None);
    };
    if let Some(c) = VALIDATED[i].get() {
        return Ok(Some(c.clone()));
    }
    let canonical = helper::validate(pack.bytes, opts)?;
    if canonical.bytes != pack.bytes {
        return Err(HelperError::NotTheBuiltin(skin_id.to_owned()));
    }
    Ok(Some(VALIDATED[i].get_or_init(|| canonical).clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate;

    #[test]
    fn every_pack_is_canonical_and_the_default_is_one() {
        for pack in &PACKS {
            let c = validate::run(pack.bytes).unwrap();
            // The committed pack is the pipeline's fixed point, so what is
            // served is byte for byte what was imported.
            assert_eq!(c.bytes, pack.bytes, "builtin:{}", pack.id);
        }
        assert_eq!(find(DEFAULT).map(|(i, _)| i), Some(0));
        assert!(find("builtin:xfire-classic").is_some());
        assert!(find("builtin:combat").is_some());
        assert!(find("builtin:nope").is_none());
        assert!(find("symbiosis").is_none());
        assert!(find("pkg:symbiosis").is_none());
        for (i, pack) in PACKS.iter().enumerate() {
            let address = validate::run(pack.bytes).unwrap().address();
            assert_eq!(by_address(&address).map(|(j, _)| j), Some(i));
        }
        assert!(by_address(&"0".repeat(64)).is_none());
    }

    /// Xfire's own packs, first in [`PACKS`]; the rest are fan skins.
    const XFIRE_OWN: usize = 3;

    /// Plan E7.7.3d: Xfire's own built-ins credit Xfire, Inc. and link
    /// their source; every fan built-in credits the authors its own files
    /// name, never Xfire.
    #[test]
    fn every_pack_credits_its_authors() {
        for (i, pack) in PACKS.iter().enumerate() {
            let m = crate::manifest::parse(&crate::package::read(pack.bytes).unwrap()).unwrap();
            let c = m
                .credits
                .unwrap_or_else(|| panic!("builtin:{} has no credits", pack.id));
            if i < XFIRE_OWN {
                assert_eq!(c.authors, ["Xfire, Inc."], "builtin:{}", pack.id);
                assert!(c.url.is_some(), "builtin:{}", pack.id);
            } else {
                assert!(!c.authors.is_empty(), "builtin:{}", pack.id);
                assert!(
                    !c.authors.iter().any(|a| a.contains("Xfire")),
                    "builtin:{}",
                    pack.id
                );
            }
        }
    }

    /// Plan E7.7.3e: a fan built-in carries its author's art, at least one
    /// image region, never only a palette.
    #[test]
    fn every_fan_pack_carries_its_art() {
        for pack in &PACKS[XFIRE_OWN..] {
            let m = crate::manifest::parse(&crate::package::read(pack.bytes).unwrap()).unwrap();
            assert!(!m.regions.is_empty(), "builtin:{} is colour-only", pack.id);
        }
    }
}
