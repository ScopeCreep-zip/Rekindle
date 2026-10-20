//! The 1:1 send path on both hosts (plan E2.1, on `DmFrame` since E2.3):
//! sealing by carrier (`frame`), sends over the ports with their fallbacks,
//! the friend-handshake messages, our direct messages and our typing.

mod chat;
pub mod frame;
mod outgoing;
pub mod route;
mod send;
mod typing;

pub use chat::{
    client_id_text, history, inbox, mark_read, parse_client_id, resend_dm, resend_peer, send_dm,
};
pub(crate) use chat::{count_unread, NewDmFields};
pub use frame::wire;
pub use outgoing::{send_friend_accept, send_friend_reject, send_friend_request};
pub use send::{
    build_and_queue, hint_for, resend, send_frame, send_to_peer, send_to_peer_call, Queued, Resend,
};
pub use typing::report_draft;

/// The ingress tag of a 1:1 message: the first byte of every 1:1 datagram
/// (§6 ingress ledger, `rekindle_codec::ingress`, beside the gossip tag),
/// so a receiver tells it from gossip and a transport frame without trying
/// to decode it.
pub use rekindle_codec::ingress::DIRECT_TAG;

/// The recipient's identity key a frame to `to` names.
///
/// # Errors
/// `Invalid` when `to` is not a public key.
pub(crate) fn recipient_key(to: &str) -> Result<[u8; 32], crate::operations::friend::FriendError> {
    rekindle_types::key_format::public_key_hex(to)
        .map(|k| k.to_bytes())
        .map_err(|e| {
            crate::operations::friend::FriendError::Invalid(format!("recipient key {to}: {e}"))
        })
}

/// The frame bytes of a 1:1 datagram, when it carries [`DIRECT_TAG`].
#[must_use]
pub fn unwire(datagram: &[u8]) -> Option<&[u8]> {
    match datagram.split_first() {
        Some((&DIRECT_TAG, rest)) => Some(rest),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use rekindle_codec::capnp_codec::direct::{
        DmFrame, DmTypeId, FrameBody, RatchetBody, RatchetHeader,
    };

    use super::*;

    fn frame(ciphertext: Vec<u8>) -> DmFrame {
        DmFrame {
            type_id: DmTypeId::DmText,
            sender: [1; 32],
            body: FrameBody::Ratchet(RatchetBody {
                header: RatchetHeader {
                    dh: [2; 32],
                    pn: 0,
                    n: 3,
                },
                ciphertext,
            }),
        }
    }

    #[test]
    fn a_wired_frame_unwires_to_its_encoding() {
        let sent = frame(b"hi".to_vec());
        let datagram = wire(&sent).unwrap();
        assert_eq!(datagram[0], DIRECT_TAG);
        assert_eq!(DmFrame::decode(unwire(&datagram).unwrap()).unwrap(), sent);
        assert_eq!(unwire(&[0x01, 2, 3]), None);
        assert_eq!(unwire(&[]), None);
    }

    #[test]
    fn a_frame_over_the_cap_is_refused() {
        // Packed Cap'n Proto shrinks zeros; these bytes do not pack.
        let big = frame(vec![0x5A; rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN]);
        assert!(matches!(
            wire(&big),
            Err(crate::operations::friend::FriendError::Invalid(_))
        ));
    }
}
