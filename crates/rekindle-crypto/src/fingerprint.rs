//! Safety numbers (plan E2.3): Signal's numeric fingerprint, the 60
//! digits two people compare out of band, and its scannable (QR) form.
//!
//! Each side's fingerprint is an iterated SHA-512 over a version, the
//! identity key and a stable identifier: `H0 = SHA-512(0x0000 ‖ key ‖ id ‖
//! key)`, `Hi = SHA-512(Hi-1 ‖ key)`, [`ITERATIONS`] hashes in all. Its
//! displayable form is the first 30 bytes as six 5-byte big-endian chunks,
//! each `mod 100000` as five digits; the two halves are joined lower first,
//! so both peers show the same 60 digits. The scannable form is the
//! `CombinedFingerprints` protobuf (`version`, then the first 32 bytes of
//! our fingerprint and of theirs), which a scanner compares crossed. This
//! is libsignal's `fingerprint.rs` (`Fingerprint::get_fingerprint`,
//! `DisplayableFingerprint`, `ScannableFingerprint`) and
//! `proto/fingerprint.proto`; the tests run its published vectors.
//!
//! Rekindle's inputs: the key is `EncodeEC` of the Ed25519 identity
//! (`0x06 ‖ key`, plan E2.2), and the stable identifier is the identity
//! key itself, because the identity is the address.

use sha2::{Digest, Sha512};

use crate::signal::pqxdh::encode::{ED25519_TYPE, ENCODED_EC_LEN};

/// Hash iterations per side (libsignal's and Signal's apps' 5200).
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
    /// The bytes are not a `CombinedFingerprints` of two 32-byte halves.
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

    /// The QR payload: `CombinedFingerprints{version, local, remote}`.
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

/// libsignal's `get_fingerprint`: the iterated SHA-512 of one side.
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

/// The protobuf encoding of `CombinedFingerprints` (fields 1 `version`,
/// 2 and 3 `LogicalFingerprint{content = 1}`), in field order.
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

/// One `LogicalFingerprint` field holding exactly 32 content bytes.
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

/// Equality that reads every byte (libsignal compares with `ct_eq`).
fn equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    // libsignal `rust/protocol/src/fingerprint.rs` test vectors
    // (`testVectorsVersion1`/`2` in its Java port): Curve25519 identity
    // keys in libsignal's `0x05 ‖ key` encoding and phone-number ids.
    const ALICE_IDENTITY: &str =
        "0506863bc66d02b40d27b8d49ca7c09e9239236f9d7d25d6fcca5ce13c7064d868";
    const BOB_IDENTITY: &str = "05f781b6fb32fed9ba1cf2de978d4d5da28dc34046ae814402b5c0dbd96fda907b";
    const ALICE_STABLE_ID: &[u8] = b"+14152222222";
    const BOB_STABLE_ID: &[u8] = b"+14153333333";
    const DISPLAYABLE: &str = "300354477692869396892869876765458257569162576843440918079131";
    const ALICE_SCANNABLE_V1: &str = "080112220a201e301a0353dce3dbe7684cb8336e85136cdc0ee96219494ada305d62a7bd61df1a220a20d62cbf73a11592015b6b9f1682ac306fea3aaf3885b84d12bca631e9d4fb3a4d";
    const BOB_SCANNABLE_V1: &str = "080112220a20d62cbf73a11592015b6b9f1682ac306fea3aaf3885b84d12bca631e9d4fb3a4d1a220a201e301a0353dce3dbe7684cb8336e85136cdc0ee96219494ada305d62a7bd61df";
    const ALICE_SCANNABLE_V2: &str = "080212220a201e301a0353dce3dbe7684cb8336e85136cdc0ee96219494ada305d62a7bd61df1a220a20d62cbf73a11592015b6b9f1682ac306fea3aaf3885b84d12bca631e9d4fb3a4d";

    fn libsignal_sides() -> ([u8; 64], [u8; 64]) {
        let alice = hex::decode(ALICE_IDENTITY).unwrap();
        let bob = hex::decode(BOB_IDENTITY).unwrap();
        (
            iterated(ITERATIONS, ALICE_STABLE_ID, &alice),
            iterated(ITERATIONS, BOB_STABLE_ID, &bob),
        )
    }

    #[test]
    fn libsignal_vectors_reproduce() {
        let (alice, bob) = libsignal_sides();
        assert_eq!(joined(&alice, &bob), DISPLAYABLE);
        assert_eq!(joined(&bob, &alice), DISPLAYABLE);
        assert_eq!(hex::encode(scannable(1, &alice, &bob)), ALICE_SCANNABLE_V1);
        assert_eq!(hex::encode(scannable(1, &bob, &alice)), BOB_SCANNABLE_V1);
        assert_eq!(hex::encode(scannable(2, &alice, &bob)), ALICE_SCANNABLE_V2);
    }

    #[test]
    fn the_scannable_encoding_matches_libsignal() {
        // libsignal `fingerprint_encodings`.
        let (mut l, mut r) = ([0u8; 64], [0u8; 64]);
        l[..32].fill(0x12);
        r[..32].fill(0xba);
        let expected = format!("080212220a20{}1a220a20{}", "12".repeat(32), "ba".repeat(32));
        assert_eq!(hex::encode(scannable(2, &l, &r)), expected);
        assert_eq!(scannable(2, &l, &r).len(), SCANNABLE_LEN);
    }

    #[test]
    fn both_peers_see_the_same_digits_and_each_scan_matches_crossed() {
        let (alice, bob) = ([1u8; 32], [2u8; 32]);
        let ours = SafetyNumber::new(&alice, &bob);
        let theirs = SafetyNumber::new(&bob, &alice);
        assert_eq!(ours.digits(), theirs.digits());
        assert_eq!(ours.digits().len(), 2 * DIGITS_PER_SIDE);
        assert!(ours.digits().bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(ours.matches_scanned(&theirs.scannable()), Ok(true));
        assert_eq!(theirs.matches_scanned(&ours.scannable()), Ok(true));
        assert_eq!(ours.matches_scanned(&ours.scannable()), Ok(false));
    }

    #[test]
    fn a_substituted_key_changes_the_number_and_fails_the_scan() {
        let (alice, bob, mallory) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        let alice_view = SafetyNumber::new(&alice, &mallory);
        let bob_view = SafetyNumber::new(&bob, &alice);
        assert_ne!(alice_view.digits(), bob_view.digits());
        assert_eq!(alice_view.matches_scanned(&bob_view.scannable()), Ok(false));
    }

    #[test]
    fn a_malformed_or_other_version_code_is_refused() {
        let number = SafetyNumber::new(&[1; 32], &[2; 32]);
        let code = SafetyNumber::new(&[2; 32], &[1; 32]).scannable();
        let mut v1 = code.clone();
        v1[1] = 1;
        assert_eq!(
            number.matches_scanned(&v1),
            Err(FingerprintError::VersionMismatch { theirs: 1, ours: 2 })
        );
        assert_eq!(
            number.matches_scanned(&code[..code.len() - 1]),
            Err(FingerprintError::Malformed)
        );
        let mut trailing = code.clone();
        trailing.push(0);
        assert_eq!(
            number.matches_scanned(&trailing),
            Err(FingerprintError::Malformed)
        );
        assert_eq!(
            number.matches_scanned(&[]),
            Err(FingerprintError::Malformed)
        );
        let (l, r) = libsignal_sides();
        let wide = scannable(300, &l, &r);
        assert_eq!(parse_scannable(&wide).map(|p| p.0), Ok(300));
    }
}
