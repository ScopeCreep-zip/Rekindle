//! Media datagram framing on a peer's media route (plan E4.3.1).
//!
//! Every datagram starts with a one-byte tag. Media that the bandwidth
//! estimator counts — voice packets and video-plane envelopes — then carry
//! the route's `transport_seq` (u32, little-endian), stamped when the
//! packet leaves for that peer: libwebrtc's `PacketRouter` populates the
//! transport-wide sequence number at send time, per transport
//! (`evidence/e4-3-bandwidth-owner-design.md` #2). It sits outside SFrame
//! and the packet signature because one sealed frame goes to every peer
//! with a different sequence number per route; Veilid's end-to-end route
//! AEAD keeps relays from altering it (#3), the integrity SRTP gives an
//! RTP header extension.
//!
//! Padding (`PADDING_TAG`) is sequenced too: the estimator counts it.
//! Feedback and receiver reports are control, not counted media, and
//! carry no sequence number.

/// A voice packet (`VoicePacket`, Cap'n Proto) after the sequence number.
pub const VOICE_TAG: u8 = b'V';
/// A signed community envelope of the video plane (video fragments,
/// parity, frame acks, keyframe requests) after the sequence number.
pub const ENVELOPE_TAG: u8 = b'M';
/// Padding the pacer sends to probe the route (plan E4.3.3): zero bytes
/// after the sequence number, counted by the estimator, discarded on
/// arrival. str0m sends blank padding the same way (`poll_packet_padding`).
pub const PADDING_TAG: u8 = b'P';
/// A `TransportFeedback` report (plan E4.3.2); no sequence number.
pub const FEEDBACK_TAG: u8 = b'T';
/// Tag plus sequence number.
pub const SEQUENCED_HEADER_LEN: usize = 5;

/// A padding payload starts with the sender's key and its Ed25519
/// signature over [`padding_signing_bytes`]; the rest is fill. Padding is
/// counted by the receiver's feedback, so it has to say whose route
/// sequence it fills, and the signature binds that sequence number: a
/// party holding our route cannot replay it under another.
pub const PADDING_HEADER_LEN: usize = 32 + 64;

/// What a padding datagram's sender signs.
#[must_use]
pub fn padding_signing_bytes(sender_key: &[u8], transport_seq: u32) -> Vec<u8> {
    let domain = rekindle_types::domains::MEDIA_PADDING.as_bytes();
    let mut out = Vec::with_capacity(domain.len() + sender_key.len() + 4);
    out.extend_from_slice(domain);
    out.extend_from_slice(sender_key);
    out.extend_from_slice(&transport_seq.to_le_bytes());
    out
}

/// Sign a sequenced padding datagram in place. `false` when it is not
/// padding or too short to carry the header.
pub fn sign_padding(datagram: &mut [u8], signing_key: &ed25519_dalek::SigningKey) -> bool {
    use ed25519_dalek::Signer;
    let Some((PADDING_TAG, seq, payload)) = split_sequenced(datagram) else {
        return false;
    };
    if payload.len() < PADDING_HEADER_LEN {
        return false;
    }
    let sender_key = signing_key.verifying_key().to_bytes();
    let sig = signing_key.sign(&padding_signing_bytes(&sender_key, seq));
    let header = &mut datagram[SEQUENCED_HEADER_LEN..SEQUENCED_HEADER_LEN + PADDING_HEADER_LEN];
    header[..32].copy_from_slice(&sender_key);
    header[32..].copy_from_slice(&sig.to_bytes());
    true
}

/// The sender of a padding payload that arrived as `transport_seq`, if its
/// signature holds.
#[must_use]
pub fn open_padding(transport_seq: u32, payload: &[u8]) -> Option<[u8; 32]> {
    let header = payload.get(..PADDING_HEADER_LEN)?;
    let sender: [u8; 32] = header[..32].try_into().ok()?;
    let sig: [u8; 64] = header[32..].try_into().ok()?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&sender).ok()?;
    key.verify_strict(
        &padding_signing_bytes(&sender, transport_seq),
        &ed25519_dalek::Signature::from_bytes(&sig),
    )
    .ok()?;
    Some(sender)
}

/// Frame `payload` under `tag` with the route's `transport_seq`.
#[must_use]
pub fn sequenced(tag: u8, transport_seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SEQUENCED_HEADER_LEN + payload.len());
    out.push(tag);
    out.extend_from_slice(&transport_seq.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Split a sequenced datagram (voice, envelope or padding) into its tag, sequence
/// number and payload; `None` for any other datagram.
#[must_use]
pub fn split_sequenced(message: &[u8]) -> Option<(u8, u32, &[u8])> {
    let (&tag, rest) = message.split_first()?;
    if tag != VOICE_TAG && tag != ENVELOPE_TAG && tag != PADDING_TAG {
        return None;
    }
    let (seq, payload) = rest.split_first_chunk::<4>()?;
    Some((tag, u32::from_le_bytes(*seq), payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequenced_round_trip() {
        let framed = sequenced(ENVELOPE_TAG, 0xDEAD_BEEF, b"abc");
        assert_eq!(
            split_sequenced(&framed),
            Some((ENVELOPE_TAG, 0xDEAD_BEEF, &b"abc"[..]))
        );
    }

    #[test]
    fn padding_is_bound_to_its_sender_and_sequence() {
        let key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
        let mut datagram = sequenced(PADDING_TAG, 41, &[0u8; PADDING_HEADER_LEN + 10]);
        assert!(sign_padding(&mut datagram, &key));
        let (_, seq, payload) = split_sequenced(&datagram).unwrap();
        assert_eq!(
            open_padding(seq, payload),
            Some(key.verifying_key().to_bytes())
        );
        assert_eq!(
            open_padding(42, payload),
            None,
            "replayed under another sequence"
        );
        let mut short = sequenced(PADDING_TAG, 1, &[0u8; 10]);
        assert!(!sign_padding(&mut short, &key));
        let mut voice = sequenced(VOICE_TAG, 1, &[0u8; PADDING_HEADER_LEN]);
        assert!(!sign_padding(&mut voice, &key));
    }

    #[test]
    fn other_tags_and_short_frames_are_not_sequenced() {
        assert_eq!(split_sequenced(&[FEEDBACK_TAG, 0, 0, 0, 0]), None);
        assert_eq!(split_sequenced(&[VOICE_TAG, 1, 2]), None);
        assert_eq!(split_sequenced(&[]), None);
    }
}
