//! Whose row holds a member's own presence slot after a superseded write
//! (plan C7.16, C7.17).
//!
//! A write to our registry slot that comes back superseded has met a value
//! the network held at our sequence number or above; Veilid adopted it and
//! stored it locally (`set_value.rs:620-645`). What the writer may do next
//! depends on whose it is, decided by the row's W26 signature: over our own
//! newer copy it writes again, over another member's row it never does
//! (slot keys come from the community's shared seed, so another member can
//! write our slot; ADR 0011 removes that).

use rekindle_types::presence::MemberPresence;

/// The signed row a member writes into its own slot when it leaves.
///
/// Signed, never empty bytes: the slot seed is shared, so an unsigned
/// empty payload would let anyone free anyone's slot
/// (`MemberPresence::departed`). `status` is "offline" so a reader that
/// predates `departed` still drops the member rather than showing a ghost.
#[must_use]
pub fn departure_row(
    pseudonym_signing_key: &rekindle_secrets::ed25519_dalek::SigningKey,
    now_secs: u64,
) -> Vec<u8> {
    let mut presence = MemberPresence {
        pseudonym_key: rekindle_types::id::PseudonymKey(
            pseudonym_signing_key.verifying_key().to_bytes(),
        ),
        status: "offline".into(),
        departed: true,
        last_heartbeat: now_secs,
        ..Default::default()
    };
    presence.signature = rekindle_secrets::derive::sign_with_pseudonym(
        pseudonym_signing_key,
        &presence.signing_bytes(),
    )
    .to_vec();
    serde_json::to_vec(&presence).expect("a presence row always serializes")
}

/// Whose value superseded a write to our own presence slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupersedingRow {
    /// Signed by our own pseudonym: a newer copy of our row that the
    /// network holds and our local store did not (for example, a write in
    /// flight when the process stopped).
    Ours,
    /// Validly signed by another member (hex pseudonym): that member wrote
    /// our slot.
    Member(String),
    /// Not a validly signed row; the label says why.
    Unverified(&'static str),
}

/// Classify the value that superseded a write to our slot.
#[must_use]
pub fn classify_superseding_row(raw_bytes: &[u8], my_pseudonym_hex: &str) -> SupersedingRow {
    if raw_bytes.is_empty() {
        return SupersedingRow::Unverified("empty-payload");
    }
    let Ok(presence) = serde_json::from_slice::<MemberPresence>(raw_bytes) else {
        return SupersedingRow::Unverified("malformed-json");
    };
    let Ok(sig_arr) = <[u8; 64]>::try_from(presence.signature.as_slice()) else {
        return SupersedingRow::Unverified("invalid-signature-length");
    };
    if rekindle_secrets::derive::verify_pseudonym_signature(
        &presence.pseudonym_key.0,
        &presence.signing_bytes(),
        &sig_arr,
    )
    .is_err()
    {
        return SupersedingRow::Unverified("signature-rejected");
    }
    let author = hex::encode(presence.pseudonym_key.0);
    if author == my_pseudonym_hex {
        SupersedingRow::Ours
    } else {
        SupersedingRow::Member(author)
    }
}

#[cfg(test)]
#[path = "presence_row_tests.rs"]
mod tests;
