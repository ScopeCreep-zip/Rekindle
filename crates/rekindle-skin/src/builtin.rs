//! The built-in packs (plan E7.7.2 "Built-in packs").
//!
//! Each lives in `crates/rekindle-skin/builtin/<id>.rkskin`, written by
//! `cargo xtask import-skin … --builtin <id>` (E7.7.3) beside its `NOTICE`,
//! and is embedded here with `include_bytes!`. Embedding is not trust: a
//! built-in passes the same helper validation as any import, on its first
//! use in each process ([`validated`]), and the canonical package the
//! helper returns is what is served.
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
/// id `classic` is the sound bank) and Combat.
pub const PACKS: [Builtin; 3] = [
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
];

/// Each pack's helper-validated canonical package, once per process.
static VALIDATED: [OnceLock<Canonical>; PACKS.len()] = [const { OnceLock::new() }; PACKS.len()];

/// The pack a `builtin:<id>` skin id names, and its index in [`PACKS`].
#[must_use]
pub fn find(skin_id: &str) -> Option<(usize, &'static Builtin)> {
    let id = skin_id.strip_prefix(PREFIX)?;
    PACKS.iter().enumerate().find(|(_, b)| b.id == id)
}

/// The canonical package of the built-in `skin_id`: validated by the
/// helper at `opts` on first use in this process, the result kept for
/// every later call. A failure is not kept, so a helper that did not
/// answer is tried again on the next use.
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
    }
}
