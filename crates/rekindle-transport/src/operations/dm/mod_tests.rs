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
