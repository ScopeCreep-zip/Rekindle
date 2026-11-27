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
#[path = "builtin_tests.rs"]
mod tests;
