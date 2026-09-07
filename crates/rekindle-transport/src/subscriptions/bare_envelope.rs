//! The one unframed inbound format this track accepts.
//!
//! Every other `app_call` must arrive as a transport frame carrying a
//! `SignedPayload`, and `dispatch_app_call` verifies that signature
//! before the handler sees anything. A wrapped MEK is the single
//! exception: the desktop track sends it as a bare Cap'n Proto
//! `CommunityEnvelope` (`services/veilid/network.rs` reads
//! `call.message()` and decodes it directly), so without this the two
//! shells NAK each other's format and a rotated key never crosses
//! between them.
//!
//! Admitting it is safe *because of what the payload is*, not because
//! the sender asserts anything: `rekindle_secrets::mek::unwrap_mek`
//! takes the sender's pseudonym public key as an ECDH input, so
//! ciphertext only decrypts if the named sender actually produced it.
//! The `sender_pseudonym` field is bound by the cryptography rather
//! than believed.
//!
//! That argument covers `MekTransfer` and nothing else. Every other
//! control variant carries its authority in plaintext fields — a
//! `MEKRotated` generation, a `GovernanceUpdated` subkey — and would be
//! forgeable by anyone who can reach our route. Those must keep
//! arriving signed, which is why this returns `None` for them and lets
//! the caller NAK.

use rekindle_protocol::dht::community::envelope::{
    CommunityEnvelope, ControlPayload, MekTransferPayload,
};

/// Recognise a bare `CommunityEnvelope` carrying a wrapped MEK.
///
/// A *typed* discriminator, not byte-sniffing:
/// `try_decode_community_envelope` returns `Err` on malformed bytes and
/// `Ok(None)` on a union discriminant it does not know, so anything
/// that is not genuinely this envelope falls through to `None`.
pub(super) fn decode_bare_mek_transfer(raw: &[u8]) -> Option<MekTransferPayload> {
    match rekindle_protocol::capnp_envelope::try_decode_community_envelope(raw) {
        Ok(Some(CommunityEnvelope::Control(ControlPayload::MekTransfer(transfer)))) => {
            Some(transfer)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::decode_bare_mek_transfer;
    use rekindle_protocol::capnp_envelope::encode_community_envelope;
    use rekindle_protocol::dht::community::envelope::{
        CommunityEnvelope, ControlPayload, MekTransferPayload,
    };

    fn mek_transfer_bytes() -> Vec<u8> {
        encode_community_envelope(&CommunityEnvelope::Control(ControlPayload::MekTransfer(
            MekTransferPayload {
                community_id: "gov-key".into(),
                channel_id: Some("general".into()),
                generation: 7,
                sender_pseudonym: "aa".repeat(32),
                wrapped_mek: vec![1, 2, 3, 4],
            },
        )))
        .expect("encode")
    }

    #[test]
    fn admits_a_wrapped_mek() {
        let decoded = decode_bare_mek_transfer(&mek_transfer_bytes()).expect("should admit");
        assert_eq!(decoded.community_id, "gov-key");
        assert_eq!(decoded.generation, 7);
        assert_eq!(decoded.wrapped_mek, vec![1, 2, 3, 4]);
    }

    /// The security boundary. `MEKRotated` states a generation in
    /// plaintext, so accepting it unsigned would let anyone who can
    /// reach our route claim a rotation happened. It must fall through
    /// to the NAK path even though it is a perfectly valid envelope.
    #[test]
    fn refuses_other_control_variants_even_when_well_formed() {
        let rotated =
            encode_community_envelope(&CommunityEnvelope::Control(ControlPayload::MEKRotated {
                channel_id: Some("general".into()),
                new_generation: 9,
                rotator_pseudonym: Some("bb".repeat(32)),
            }))
            .expect("encode");
        assert!(
            decode_bare_mek_transfer(&rotated).is_none(),
            "an unsigned MEKRotated must not be actionable"
        );
    }

    #[test]
    fn refuses_garbage() {
        assert!(decode_bare_mek_transfer(b"not an envelope at all").is_none());
        assert!(decode_bare_mek_transfer(&[]).is_none());
    }

    /// Truncation must not be mistaken for a valid short envelope.
    #[test]
    fn refuses_truncated_envelope() {
        let full = mek_transfer_bytes();
        for cut in [1, full.len() / 3, full.len() / 2, full.len() - 1] {
            assert!(
                decode_bare_mek_transfer(&full[..cut]).is_none(),
                "truncation at {cut} must not decode"
            );
        }
    }
}
