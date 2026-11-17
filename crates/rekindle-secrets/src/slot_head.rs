//! The digest a departure's slot head commits to (plan E3.2 S9): the exact
//! value of a departed member's channel slot, so a reader keeps its
//! backfilled posts only from that value. A leaver that signed a head above
//! the seq it had published, and later wrote into the gap (veilid-core 0.5.8
//! `set_value` takes `min_seqnum`), cannot make the gap's posts match: the
//! committed page was fixed when the departure was signed.
//!
//! `digest = BLAKE3 derive_key(SLOT_HEAD, len(record_key) ‖ record_key ‖
//! subkey ‖ seq ‖ page)`, integers little-endian, over the decrypted page
//! (record encryption re-randomises the stored bytes; every reader sees the
//! decrypted page byte-identically).

use rekindle_types::domains;

/// The digest of the slot value `seq` of `subkey` in `record_key`, whose
/// decrypted page is `page`.
#[must_use]
pub fn slot_head_digest(record_key: &str, subkey: u32, seq: u32, page: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(domains::SLOT_HEAD);
    let key_len = u64::try_from(record_key.len()).unwrap_or(u64::MAX);
    hasher.update(&key_len.to_le_bytes());
    hasher.update(record_key.as_bytes());
    hasher.update(&subkey.to_le_bytes());
    hasher.update(&seq.to_le_bytes());
    hasher.update(page);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::slot_head_digest;

    /// Every input moves the digest: a head names one record, one slot, one
    /// seq and one page, and the record key is length-prefixed so a key's
    /// tail cannot pass as the subkey.
    #[test]
    fn every_field_is_committed() {
        let base = slot_head_digest("VLD0:a", 2, 7, b"page");
        assert_eq!(base, slot_head_digest("VLD0:a", 2, 7, b"page"));
        for other in [
            slot_head_digest("VLD0:b", 2, 7, b"page"),
            slot_head_digest("VLD0:a", 3, 7, b"page"),
            slot_head_digest("VLD0:a", 2, 8, b"page"),
            slot_head_digest("VLD0:a", 2, 7, b"pagf"),
            slot_head_digest("VLD0:a\u{2}", 0, 7, b"page"),
        ] {
            assert_ne!(base, other);
        }
    }
}
