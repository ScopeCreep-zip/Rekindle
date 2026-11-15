//! Validator step 7: the output (plan E7.7.1).
//!
//! * [`package`]: an immutable, BLAKE3-addressed canonical package. The
//!   validated skin is re-zipped from what the earlier steps produced and
//!   nothing of the input archive: the manifest re-serialised from
//!   [`SkinManifest`], each variant's tokens re-emitted from their numbers
//!   as a DTCG file, the canonical images and sounds. Entries are stored
//!   (method 0), in byte order of their names, with one fixed timestamp
//!   (the MS-DOS epoch, 1980-01-01 00:00), mode 0644, no extra fields,
//!   comments or directory entries, so the same skin always yields the
//!   same bytes and the same address whatever the input's entry order.
//! * [`tokens_css`]: the `tokens.css` body a variant is applied with,
//!   `@layer skin { :root { … } }`, built by [`emit::declarations`] from
//!   the resolved numbers alone; every value is drawn from
//!   [`emit::ALPHABET`] or is an app-owned font stack, and every property
//!   name is a registry constant, so no skin string reaches CSS.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule, TokenValue};
use serde_json::{Map, Value};

use crate::limits::{MAX_ENTRIES, MAX_PACKAGE_BYTES};
use crate::package::MANIFEST;
use crate::registry::{entry_in, TokenEntry, TokenType, ENTRIES};
use crate::tokens::emit::{self, thousandths_number};

/// A canonical package and its address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canonical {
    /// The stored zip.
    pub bytes: Vec<u8>,
    /// BLAKE3 of `bytes`.
    pub hash: [u8; 32],
}

impl Canonical {
    /// The address: `hash` as 64 lowercase hex digits.
    #[must_use]
    pub fn address(&self) -> String {
        self.hash
            .iter()
            .fold(String::with_capacity(64), |mut s, b| {
                s.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
                s.push(char::from(b"0123456789abcdef"[usize::from(b & 15)]));
                s
            })
    }
}

/// MS-DOS date of 1980-01-01 (day 1, month 1, year 0).
const DOS_DATE: u16 = 0x0021;
/// Unix "made by" host, version 2.0.
const MADE_BY_UNIX: u16 = 0x0314;
/// A regular file, mode 0644, in the external attributes' high half.
const REGULAR_0644: u32 = 0o100_644 << 16;
/// General-purpose flag bit 11: the name is UTF-8.
const FLAG_UTF8: u16 = 1 << 11;

/// The DTCG `$value` of a re-emitted token.
fn dtcg_value(e: &TokenEntry, v: &TokenValue) -> Option<Value> {
    Some(match (e.ty, v) {
        (TokenType::Color, TokenValue::Color { .. }) => Value::String(emit::css_value(e, v)?),
        (TokenType::Dimension { .. }, TokenValue::Dimension(px)) => {
            serde_json::json!({ "value": px, "unit": "px" })
        }
        (TokenType::Number { .. } | TokenType::Fraction { .. }, TokenValue::Number(t)) => {
            serde_json::from_str(&thousandths_number(*t)).ok()?
        }
        (TokenType::FontFamily, TokenValue::FontFamily(name)) => Value::String(name.clone()),
        _ => return None,
    })
}

/// A variant's parsed tokens as a canonical DTCG token file: groups by the
/// id's dotted path, each token `{$type, $value}`, keys in byte order.
///
/// # Errors
/// A token the registry lacks or whose value does not fit its type
/// (never for what [`crate::tokens::parse`] returns).
pub fn token_file(
    entry: &str,
    tokens: &BTreeMap<String, TokenValue>,
) -> Result<Vec<u8>, SkinError> {
    let mut root = Map::new();
    for (id, v) in tokens {
        let bad = || SkinError::new(entry, SkinRule::UnknownToken { token: id.clone() });
        let e = entry_in(ENTRIES, id).ok_or_else(bad)?;
        let value = dtcg_value(e, v).ok_or_else(bad)?;
        let mut group = &mut root;
        let mut parts = id.split('.').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                let mut token = Map::new();
                token.insert("$type".to_owned(), Value::String(e.ty.dtcg().to_owned()));
                token.insert("$value".to_owned(), value.clone());
                group.insert(part.to_owned(), Value::Object(token));
            } else {
                let next = group
                    .entry(part.to_owned())
                    .or_insert_with(|| Value::Object(Map::new()));
                let Value::Object(next) = next else {
                    return Err(bad());
                };
                group = next;
            }
        }
    }
    serde_json::to_vec(&Value::Object(root)).map_err(|_| {
        SkinError::new(
            entry,
            SkinRule::UnknownToken {
                token: entry.to_owned(),
            },
        )
    })
}

/// A length past what a zip field holds: far over the package limit.
fn too_large(n: usize) -> SkinError {
    SkinError::new(
        SkinError::PACKAGE,
        SkinRule::PackageTooLarge {
            size: u64::try_from(n).unwrap_or(u64::MAX),
            max: MAX_PACKAGE_BYTES,
        },
    )
}

fn u16_of(n: usize) -> Result<u16, SkinError> {
    u16::try_from(n).map_err(|_| too_large(n))
}

fn u32_of(n: usize) -> Result<u32, SkinError> {
    u32::try_from(n).map_err(|_| too_large(n))
}

/// The fixed header fields both records share after the signature and the
/// version(s): flags, method 0, time 0, the date, CRC-32 and both sizes.
fn common_fields(out: &mut Vec<u8>, flags: u16, crc: u32, size: u32) {
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&DOS_DATE.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
}

/// `files` as a stored zip (APPNOTE 6.3.10 §4.3), in key order.
fn zip(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, SkinError> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = u32_of(out.len())?;
        let flags = if name.is_ascii() { 0 } else { FLAG_UTF8 };
        let crc = crc32fast::hash(data);
        let size = u32_of(data.len())?;
        let name_len = u16_of(name.len())?;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        common_fields(&mut out, flags, crc, size);
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&MADE_BY_UNIX.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        common_fields(&mut central, flags, crc, size);
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
        central.extend_from_slice(&REGULAR_0644.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let count = u16_of(files.len())?;
    let cd_offset = u32_of(out.len())?;
    let cd_size = u32_of(central.len())?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // disk numbers
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

/// The canonical package of a validated skin: `manifest`, each variant's
/// parsed `tokens` (by the variant's token file name), and the canonical
/// `images` and `sounds` of steps 4 and 5.
///
/// # Errors
/// A variant without tokens, a token that does not re-emit, or a result
/// over the package size or entry limits.
pub fn package(
    manifest: &SkinManifest,
    tokens: &BTreeMap<String, BTreeMap<String, TokenValue>>,
    images: &BTreeMap<String, Vec<u8>>,
    sounds: &BTreeMap<String, Vec<u8>>,
) -> Result<Canonical, SkinError> {
    let mut files = BTreeMap::new();
    let json = serde_json::to_vec(manifest).map_err(|e| {
        SkinError::new(
            MANIFEST,
            SkinRule::ManifestSyntax {
                detail: e.to_string(),
            },
        )
    })?;
    files.insert(MANIFEST.to_owned(), json);
    for v in &manifest.variants {
        let parsed = tokens.get(&v.tokens).ok_or_else(|| {
            SkinError::new(
                v.tokens.as_str(),
                SkinRule::MissingEntry {
                    field: "variants.tokens".to_owned(),
                },
            )
        })?;
        files.insert(v.tokens.clone(), token_file(&v.tokens, parsed)?);
    }
    for (name, data) in images.iter().chain(sounds) {
        files.insert(name.clone(), data.clone());
    }
    let count = files.len() as u64;
    if count > MAX_ENTRIES {
        return Err(SkinError::new(
            SkinError::PACKAGE,
            SkinRule::TooManyEntries {
                count,
                max: MAX_ENTRIES,
            },
        ));
    }
    let bytes = zip(&files)?;
    let size = bytes.len() as u64;
    if size > MAX_PACKAGE_BYTES {
        return Err(SkinError::new(
            SkinError::PACKAGE,
            SkinRule::PackageTooLarge {
                size,
                max: MAX_PACKAGE_BYTES,
            },
        ));
    }
    let hash = *blake3::hash(&bytes).as_bytes();
    Ok(Canonical { bytes, hash })
}

/// The `tokens.css` body of a resolved variant: `@layer skin { :root { … } }`.
/// `None` when a pair does not fit, which [`crate::registry::resolve`]'s
/// output never does.
#[must_use]
pub fn tokens_css(resolved: &[(&TokenEntry, TokenValue)]) -> Option<String> {
    let decls = emit::declarations(resolved, 4)?;
    Some(format!("@layer skin {{\n  :root {{\n{decls}  }}\n}}\n"))
}
