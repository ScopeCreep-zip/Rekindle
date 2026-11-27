//! An image the `skin://` scheme serves by its BLAKE3 address
//! ([`super::File::Image`]): one of the package's own files, the still of
//! an animated region image, or a Disabled copy the serve made.

use rekindle_types::dto::skin::SkinError;

use super::{files, unreadable};
use crate::manifest;
use crate::package::{self, Package};
use crate::regions::still;

/// The `ext` image of the canonical package `bytes` at `address` whose
/// bytes have address `hash`; `Ok(None)` for one it neither holds nor makes.
pub(super) fn named(
    bytes: &[u8],
    address: &str,
    hash: &str,
    ext: &str,
) -> Result<Option<Vec<u8>>, SkinError> {
    let p = package::read(bytes)?;
    let found = p
        .files()
        .find(|(name, data)| {
            name.rsplit_once('.').map(|(_, e)| e) == Some(ext)
                && blake3::hash(data).to_hex().as_str() == hash
        })
        .map(|(_, data)| data.to_vec());
    Ok(match found {
        Some(bytes) => Some(bytes),
        None if ext == "png" => match still_named(&p, hash)? {
            Some(still) => Some(still),
            None => made_named(bytes, &p, address, hash)?,
        },
        None => None,
    })
}

/// The still (`skin_css`'s reduced-motion first frame, [`still::stills`])
/// of the package's animated region image whose PNG has address `hash`.
fn still_named(p: &Package<'_>, hash: &str) -> Result<Option<Vec<u8>>, SkinError> {
    let m = manifest::parse(p)?;
    let stills = still::stills(&m, &files(p)).map_err(|e| unreadable(&e.to_string()))?;
    Ok(stills
        .into_values()
        .find(|png| blake3::hash(png).to_hex().as_str() == hash))
}

/// The Disabled copy the serve made ([`crate::skin_tree::dim`]) whose PNG
/// has address `hash`, made as [`super::stylesheet`] makes it.
fn made_named(
    bytes: &[u8],
    p: &Package<'_>,
    address: &str,
    hash: &str,
) -> Result<Option<Vec<u8>>, SkinError> {
    let (_, made) = crate::conform::served(bytes, p, address)?;
    Ok(made
        .into_values()
        .find(|png| blake3::hash(png).to_hex().as_str() == hash))
}
