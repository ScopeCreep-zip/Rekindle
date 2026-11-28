//! `icons.dll` read as PE resource data with `pelite`: the file is parsed
//! from its bytes, never loaded or executed.
//!
//! Evidence from the data (Xfire build 46139): the resource directory
//! holds three types, `RT_VERSION` (16), `RT_MANIFEST` (24) and one named
//! type `ICONS`. There is no `RT_GROUP_ICON` or `RT_ICON`: each of the
//! 3,845 `ICONS` entries is a whole `.ICO` file (ICONDIR, its
//! ICONDIRENTRY table with file offsets, then the images: the file form of
//! the same group and icon records), named `XF_<SHORTNAME>.ICO` where
//! `<SHORTNAME>` is a game's `ShortName` in `xfire_games.ini`, upper-cased
//! (`XF_AAO.ICO` is section `[2]`'s `ShortName=aao`).

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, ensure, Context, Result};
use pelite::pe32::{Pe, PeFile};
use pelite::resources::Name;

use super::icon::MAX_ICO_BYTES;

/// The named resource type holding the game icons.
const ICONS_TYPE: &str = "ICONS";
/// The most icon resources read.
const MAX_ICONS: usize = 8192;
/// The largest DLL read.
pub const MAX_DLL_BYTES: usize = 64 * 1024 * 1024;

/// Every `XF_<SHORTNAME>.ICO` resource's `.ICO` bytes, keyed by
/// `<SHORTNAME>` as stored (upper case).
///
/// # Errors
/// A malformed PE or resource directory, a resource past the limits, or a
/// name not of the form `XF_<A-Z 0-9 _ ->.ICO`.
pub fn icons(dll: &[u8]) -> Result<BTreeMap<String, &[u8]>> {
    ensure!(
        dll.len() <= MAX_DLL_BYTES,
        "icons.dll of {} bytes",
        dll.len()
    );
    let file = PeFile::from_bytes(dll).map_err(|e| anyhow!("icons.dll: PE: {e}"))?;
    let resources = file
        .resources()
        .map_err(|e| anyhow!("icons.dll: resources: {e}"))?;
    let dir = resources
        .root()
        .map_err(|e| anyhow!("icons.dll: resource root: {e}"))?
        .get_dir(Name::Str(ICONS_TYPE))
        .map_err(|e| anyhow!("icons.dll: no {ICONS_TYPE} resource type: {e}"))?;
    let mut out = BTreeMap::new();
    for entry in dir.entries() {
        ensure!(out.len() < MAX_ICONS, "more than {MAX_ICONS} icons");
        let name = match entry.name().map_err(|e| anyhow!("icon name: {e}"))? {
            Name::Wide(w) => String::from_utf16(w).context("icon name is not UTF-16")?,
            other => bail!("icon resource {other} is not named"),
        };
        let short = name
            .strip_prefix("XF_")
            .and_then(|s| s.strip_suffix(".ICO"))
            .filter(|s| {
                !s.is_empty()
                    && s.bytes().all(|c| {
                        c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b'-'
                    })
            })
            .with_context(|| format!("icon resource {name} is not XF_<SHORTNAME>.ICO"))?;
        let bytes = entry
            .entry()
            .map_err(|e| anyhow!("{name}: {e}"))?
            .dir()
            .with_context(|| format!("{name}: no language directory"))?
            .first_data()
            .map_err(|e| anyhow!("{name}: {e}"))?
            .bytes()
            .map_err(|e| anyhow!("{name}: {e}"))?;
        ensure!(
            bytes.len() <= MAX_ICO_BYTES,
            "{name}: {} bytes",
            bytes.len()
        );
        ensure!(
            out.insert(short.to_owned(), bytes).is_none(),
            "{name} twice"
        );
    }
    Ok(out)
}
