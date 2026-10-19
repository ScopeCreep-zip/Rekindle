//! The 1:1 send path on both hosts (plan E2.1): sealing, the `'D'` wire,
//! sends over the ports with their fallbacks, the friend-handshake
//! messages, and our direct messages.

mod chat;
mod outgoing;
pub mod route;
mod seal;
mod send;

use rekindle_codec::message::envelope::MessageEnvelope;

use crate::operations::friend::FriendError;

pub use chat::{history, inbox, mark_read, send_dm};
pub use outgoing::{send_friend_accept, send_friend_reject, send_friend_request, send_typing};
pub(crate) use seal::recipient_key;
pub use seal::resign_for_retry;
pub use send::{build_and_queue, resend, send_to_peer, send_to_peer_call, Resend};

/// The ingress tag of a 1:1 message: the first byte of every 1:1 datagram
/// (§6 ingress ledger, `rekindle_codec::ingress`, beside the gossip tag),
/// so a receiver tells it from gossip and a transport frame without trying
/// to decode it.
pub use rekindle_codec::ingress::DIRECT_TAG;

/// The datagram for `envelope`: [`DIRECT_TAG`] then its Cap'n Proto
/// encoding. Every 1:1 send builds its bytes here.
///
/// # Errors
/// `Invalid` when the datagram is over Veilid's message cap: it fails on
/// every path, so it is refused rather than queued (plan C7.25).
pub fn wire(envelope: &MessageEnvelope) -> Result<Vec<u8>, FriendError> {
    let encoded = rekindle_codec::capnp_codec::message::encode_envelope(envelope);
    let mut datagram = Vec::with_capacity(encoded.len() + 1);
    datagram.push(DIRECT_TAG);
    datagram.extend_from_slice(&encoded);
    rekindle_records::port::check_message_len(&datagram)
        .map_err(|e| FriendError::Invalid(e.to_string()))?;
    Ok(datagram)
}

/// The envelope bytes of a 1:1 datagram, when it carries [`DIRECT_TAG`].
#[must_use]
pub fn unwire(datagram: &[u8]) -> Option<&[u8]> {
    match datagram.split_first() {
        Some((&DIRECT_TAG, rest)) => Some(rest),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wired_envelope_unwires_to_its_encoding() {
        let envelope = MessageEnvelope {
            sender_key: vec![1; 32],
            timestamp: 7,
            nonce: vec![2; 16],
            payload: b"hi".to_vec(),
            signature: vec![3; 64],
        };
        let datagram = wire(&envelope).unwrap();
        assert_eq!(datagram[0], DIRECT_TAG);
        let decoded =
            rekindle_codec::capnp_codec::message::decode_envelope(unwire(&datagram).unwrap())
                .unwrap();
        assert_eq!(decoded.payload, envelope.payload);
        assert_eq!(unwire(&[0x01, 2, 3]), None);
        assert_eq!(unwire(&[]), None);
    }

    #[test]
    fn an_envelope_over_the_cap_is_refused() {
        let envelope = MessageEnvelope {
            sender_key: vec![1; 32],
            timestamp: 7,
            nonce: vec![2; 16],
            // Packed Cap'n Proto shrinks zeros; these bytes do not pack.
            payload: vec![0x5A; rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN],
            signature: vec![3; 64],
        };
        assert!(matches!(wire(&envelope), Err(FriendError::Invalid(_))));
    }
}
