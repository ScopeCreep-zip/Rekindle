use super::*;

pub(in crate::capnp_codec::direct) fn ratchet(ciphertext_len: usize) -> RatchetBody {
    RatchetBody {
        header: RatchetHeader {
            dh: [0x11; 32],
            pn: u32::MAX,
            n: u32::MAX,
        },
        ciphertext: vec![0x5A; ciphertext_len],
    }
}

pub(in crate::capnp_codec::direct) fn init(
    first_type_id: DmTypeId,
    ciphertext_len: usize,
) -> PreKeyInitBody {
    PreKeyInitBody {
        base_key: [0x22; 32],
        signed_prekey_id: u32::MAX,
        one_time_prekey_id: Some(u32::MAX),
        pq_prekey: PqPrekey::OneTime(u32::MAX),
        kem_ciphertext: vec![0x33; super::super::tests::ML_KEM_CIPHERTEXT_LEN],
        first_type_id,
        message: ratchet(ciphertext_len),
    }
}

fn sealed() -> SealedBody {
    SealedBody {
        enc: [0x44; 32],
        ciphertext: vec![0x55; 40],
        signature: [0x66; 64],
    }
}

fn frame(type_id: DmTypeId, body: FrameBody) -> DmFrame {
    DmFrame {
        type_id,
        sender: [0x77; 32],
        body,
    }
}

#[test]
fn every_body_shape_round_trips() {
    let mut last_resort = init(DmTypeId::SessionResetAccept, 64);
    last_resort.one_time_prekey_id = None;
    last_resort.pq_prekey = PqPrekey::LastResort(3);
    for f in [
        frame(DmTypeId::DmText, FrameBody::Ratchet(ratchet(48))),
        frame(
            DmTypeId::PreKeyInit,
            FrameBody::PreKeyInit(init(DmTypeId::FriendAccept, 64)),
        ),
        frame(DmTypeId::PreKeyInit, FrameBody::PreKeyInit(last_resort)),
        frame(DmTypeId::FriendRequest, FrameBody::Sealed(sealed())),
    ] {
        assert_eq!(DmFrame::decode(&f.encode().unwrap()).unwrap(), f);
    }
}

#[test]
fn a_body_the_carrier_does_not_name_is_refused() {
    for f in [
        frame(DmTypeId::DmText, FrameBody::Sealed(sealed())),
        frame(DmTypeId::FriendRequest, FrameBody::Ratchet(ratchet(8))),
        frame(DmTypeId::FriendAccept, FrameBody::Ratchet(ratchet(8))),
        frame(
            DmTypeId::DmText,
            FrameBody::PreKeyInit(init(DmTypeId::DmText, 8)),
        ),
        frame(
            DmTypeId::PreKeyInit,
            FrameBody::PreKeyInit(init(DmTypeId::FriendRequest, 8)),
        ),
        frame(
            DmTypeId::PreKeyInit,
            FrameBody::PreKeyInit(init(DmTypeId::PreKeyInit, 8)),
        ),
    ] {
        assert!(
            matches!(
                DmFrame::decode(&f.encode().unwrap()),
                Err(CodecError::Deserialization(_))
            ),
            "{f:?}"
        );
    }
}

#[test]
fn an_unallocated_type_id_or_a_short_key_is_refused() {
    let bytes = encode_root::<dm_frame::Owned>(|mut root| {
        root.set_type_id(0x0109);
        root.set_sender(&[1; 32]);
        write_ratchet(root.init_body().init_ratchet(), &ratchet(8));
        Ok(())
    });
    assert!(matches!(
        DmFrame::decode(&bytes.unwrap()),
        Err(CodecError::UnknownVariant(_))
    ));
    let bytes = encode_root::<dm_frame::Owned>(|mut root| {
        root.set_type_id(DmTypeId::DmText.as_u16());
        root.set_sender(&[1; 31]);
        write_ratchet(root.init_body().init_ratchet(), &ratchet(8));
        Ok(())
    });
    assert!(DmFrame::decode(&bytes.unwrap()).is_err());
}

/// G6: a frame over the message cap is refused unread, and a packed
/// zero run that would unpack past the word cap is refused.
#[test]
fn oversized_and_inflating_frames_are_refused() {
    let over = vec![0u8; MAX_APP_MESSAGE_LEN];
    assert!(matches!(
        DmFrame::decode(&over),
        Err(CodecError::Deserialization(_))
    ));
    let packed = crate::capnp_codec::bounds::inflating_message(
        crate::capnp_codec::bounds::DM_FRAME_BOUND.max_words + 1,
    );
    assert!(DmFrame::decode(&packed).is_err());
    assert!(DmContent::decode(&packed).is_err());
}

#[test]
fn the_plaintexts_round_trip() {
    for content_hint in [
        ContentHint::Default,
        ContentHint::Resendable,
        ContentHint::Implicit,
    ] {
        let content = DmContent {
            stream_seq: 0,
            chain_seq: 0,
            content_hint,
            sent_at_ms: u64::MAX,
            payload: vec![1, 2, 3],
        };
        assert_eq!(DmContent::decode(&content.encode().unwrap()).unwrap(), content);
    }
    let sealed = SealedContent {
        nonce: [9; 16],
        sent_at_ms: 1_700_000_000_000,
        payload: vec![4, 5],
    };
    assert_eq!(SealedContent::decode(&sealed.encode().unwrap()).unwrap(), sealed);
}
