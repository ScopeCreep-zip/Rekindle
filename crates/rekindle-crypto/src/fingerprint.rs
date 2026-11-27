//! Safety numbers (plan E2.3): Signal's safety-number scheme, the 60
//! digits two people compare out of band, and its scannable (QR) form.
//!
//! Each side's fingerprint is an iterated SHA-512 over a two-byte version
//! (zero), the side's identity key and its stable identifier:
//! `H1 = SHA-512(0x0000 ‖ key ‖ id ‖ key)`, then
//! `Hi = SHA-512(Hi-1 ‖ key)` up to `H5200` ([`ITERATIONS`] hashes in
//! all). Its displayable form is the first 30 bytes of the result as six
//! 5-byte big-endian integers, each reduced `mod 100000` and written as
//! five zero-padded digits; the two sides' 30-digit strings are joined
//! lower first, so both peers show the same 60 digits. The scannable form
//! is a protobuf message (field 1 the varint version, fields 2 and 3 each
//! a nested message whose field 1 holds 32 bytes): the first 32 bytes of
//! our fingerprint, then of theirs. A scanner compares it crossed: the
//! code's first half against its own remote side and the second against
//! its own local side.
//!
//! Rekindle's inputs: the key is `EncodeEC` of the Ed25519 identity
//! (`0x06 ‖ key`, plan E2.2), and the stable identifier is the identity
//! key itself, because the identity is the address.

use sha2::{Digest, Sha512};

use crate::signal::pqxdh::encode::{ED25519_TYPE, ENCODED_EC_LEN};

/// Hash iterations per side (the scheme's 5200).
pub const ITERATIONS: u32 = 5200;

/// The scannable form's version: Signal's current one (2), whose stable
/// identifier is the account's service id rather than a phone number.
/// Rekindle's identifier is the identity key, which is the same kind of
/// stable, non-phone identifier.
pub const SCANNABLE_VERSION: u32 = 2;

/// Digits per side of the displayable number.
pub const DIGITS_PER_SIDE: usize = 30;

/// Bytes of each side carried in the scannable form.
const SCANNABLE_BYTES: usize = 32;

/// Length of the scannable form: `08 vv 12 22 0a 20 <32> 1a 22 0a 20 <32>`.
const SCANNABLE_LEN: usize = 2 + 2 * (4 + SCANNABLE_BYTES);

/// A scanned code that is not a safety number this build can compare.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FingerprintError {
    /// The code is another version of the format.
    #[error("safety number version {theirs}, ours is {ours}")]
    VersionMismatch { theirs: u32, ours: u32 },
    /// The bytes are not a scannable form of two 32-byte halves.
    #[error("not a scannable safety number")]
    Malformed,
}

/// The safety number of one pair of identities, from our side.
#[derive(Clone, PartialEq, Eq)]
pub struct SafetyNumber {
    local: [u8; 64],
    remote: [u8; 64],
}

impl std::fmt::Debug for SafetyNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SafetyNumber").field(&self.digits()).finish()
    }
}

impl SafetyNumber {
    /// The safety number between our identity `local` and the peer's
    /// `remote` (both Ed25519 identity keys).
    #[must_use]
    pub fn new(local: &[u8; 32], remote: &[u8; 32]) -> Self {
        Self {
            local: side(local),
            remote: side(remote),
        }
    }

    /// The 60 digits both peers see: each side's 30, lower first.
    #[must_use]
    pub fn digits(&self) -> String {
        joined(&self.local, &self.remote)
    }

    /// The QR payload: the version, our 32 bytes, then theirs.
    #[must_use]
    pub fn scannable(&self) -> Vec<u8> {
        scannable(SCANNABLE_VERSION, &self.local, &self.remote)
    }

    /// Whether a code scanned from the peer's screen shows this pair: its
    /// local half is our remote one and its remote half our local one.
    ///
    /// # Errors
    /// The code is malformed or another version.
    pub fn matches_scanned(&self, scanned: &[u8]) -> Result<bool, FingerprintError> {
        let (version, theirs_local, theirs_remote) = parse_scannable(scanned)?;
        if version != SCANNABLE_VERSION {
            return Err(FingerprintError::VersionMismatch {
                theirs: version,
                ours: SCANNABLE_VERSION,
            });
        }
        let same = equal(theirs_local, &self.remote[..SCANNABLE_BYTES])
            & equal(theirs_remote, &self.local[..SCANNABLE_BYTES]);
        Ok(same)
    }
}

/// One side's fingerprint under Rekindle's inputs.
fn side(identity: &[u8; 32]) -> [u8; 64] {
    let mut key = [0u8; ENCODED_EC_LEN];
    key[0] = ED25519_TYPE;
    key[1..].copy_from_slice(identity);
    iterated(ITERATIONS, identity, &key)
}

/// The iterated SHA-512 of one side: `H1` over the version, key, id and
/// key, then `iterations - 1` more over the previous hash and the key.
fn iterated(iterations: u32, stable_id: &[u8], key: &[u8]) -> [u8; 64] {
    let mut hash = [0u8; 64];
    hash.copy_from_slice(
        &Sha512::new()
            .chain_update([0u8, 0u8])
            .chain_update(key)
            .chain_update(stable_id)
            .chain_update(key)
            .finalize(),
    );
    for _ in 1..iterations {
        let next = Sha512::new()
            .chain_update(hash)
            .chain_update(key)
            .finalize();
        hash.copy_from_slice(&next);
    }
    hash
}

/// One side's 30 digits: six 5-byte big-endian chunks, each mod 100000.
fn side_digits(fingerprint: &[u8; 64]) -> String {
    use std::fmt::Write;
    fingerprint[..DIGITS_PER_SIDE].chunks_exact(5).fold(
        String::with_capacity(DIGITS_PER_SIDE),
        |mut out, chunk| {
            let value = chunk.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
            write!(out, "{:05}", value % 100_000).expect("writing to a String");
            out
        },
    )
}

/// Both sides' digits, the lower string first.
fn joined(local: &[u8; 64], remote: &[u8; 64]) -> String {
    let (ours, theirs) = (side_digits(local), side_digits(remote));
    if ours < theirs {
        ours + &theirs
    } else {
        theirs + &ours
    }
}

/// The scannable form's protobuf encoding (field 1 `version`, fields 2
/// and 3 a nested message holding 32 bytes in its field 1), in field
/// order.
fn scannable(version: u32, local: &[u8; 64], remote: &[u8; 64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SCANNABLE_LEN + 4);
    out.push(0x08);
    let mut value = version;
    while value >= 0x80 {
        out.push(u8::try_from(value & 0x7f).expect("seven bits") | 0x80);
        value >>= 7;
    }
    out.push(u8::try_from(value).expect("below 0x80"));
    for (field, half) in [(0x12u8, local), (0x1a, remote)] {
        let content = u8::try_from(SCANNABLE_BYTES).expect("32 fits a byte");
        out.extend_from_slice(&[field, content + 2, 0x0a, content]);
        out.extend_from_slice(&half[..SCANNABLE_BYTES]);
    }
    out
}

/// Parse [`scannable`]'s canonical form; anything else is malformed.
fn parse_scannable(bytes: &[u8]) -> Result<(u32, &[u8], &[u8]), FingerprintError> {
    let (&tag, mut rest) = bytes.split_first().ok_or(FingerprintError::Malformed)?;
    if tag != 0x08 {
        return Err(FingerprintError::Malformed);
    }
    let mut version = 0u32;
    for shift in (0..35).step_by(7) {
        let (&byte, tail) = rest.split_first().ok_or(FingerprintError::Malformed)?;
        rest = tail;
        version |= u32::from(byte & 0x7f)
            .checked_shl(shift)
            .ok_or(FingerprintError::Malformed)?;
        if byte & 0x80 == 0 {
            break;
        }
    }
    let local = half(&mut rest, 0x12)?;
    let remote = half(&mut rest, 0x1a)?;
    if !rest.is_empty() {
        return Err(FingerprintError::Malformed);
    }
    Ok((version, local, remote))
}

/// One nested half field holding exactly 32 content bytes.
fn half<'a>(rest: &mut &'a [u8], field: u8) -> Result<&'a [u8], FingerprintError> {
    let content = u8::try_from(SCANNABLE_BYTES).expect("32 fits a byte");
    let header = [field, content + 2, 0x0a, content];
    let tail = rest
        .strip_prefix(&header[..])
        .filter(|tail| tail.len() >= SCANNABLE_BYTES)
        .ok_or(FingerprintError::Malformed)?;
    let (value, after) = tail.split_at(SCANNABLE_BYTES);
    *rest = after;
    Ok(value)
}

/// Equality that reads every byte, so its time does not depend on where
/// the halves first differ.
fn equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod tests;
