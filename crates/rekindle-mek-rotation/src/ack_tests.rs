use super::*;

const INVITER: &str = "aa";
const JOINER: &str = "bb";

fn transfer() -> MekTransferPayload {
    MekTransferPayload {
        community_id: "c1".into(),
        channel_id: None,
        generation: 4,
        sender_pseudonym: INVITER.into(),
        wrapped_mek: vec![1],
        transfer_id: vec![9; 16],
    }
}

fn expected(mek: &MediaEncryptionKey) -> ExpectedAck<'_> {
    ExpectedAck {
        community_id: "c1",
        inviter_hex: INVITER,
        joiner_hex: JOINER,
        scope: KeyScope::Community,
        mek,
    }
}

fn reencode(ack: &[u8], edit: impl FnOnce(&mut MekTransferAckPayload)) -> Vec<u8> {
    let Some(ControlPayload::MekTransferAck(mut p)) =
        rekindle_codec::capnp_envelope::try_decode_community_envelope(ack)
            .ok()
            .flatten()
            .and_then(CommunityEnvelope::into_control)
    else {
        panic!("not an ack")
    };
    edit(&mut p);
    rekindle_codec::capnp_envelope::encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::MekTransferAck(p),
    ))
    .unwrap()
}

#[test]
fn the_joiners_ack_of_the_key_settles_and_nothing_else_does() {
    let mek = MediaEncryptionKey::from_bytes([7; 32], 4);
    let ack = encode_mek_transfer_ack(&transfer(), &mek, JOINER).unwrap();
    assert!(is_mek_transfer_ack(&expected(&mek), &ack));
    // A late ack of another retry (another id) of the same key counts.
    let other_id = encode_mek_transfer_ack(
        &MekTransferPayload {
            transfer_id: vec![1; 16],
            ..transfer()
        },
        &mek,
        JOINER,
    )
    .unwrap();
    assert!(is_mek_transfer_ack(&expected(&mek), &other_id));

    assert!(!is_mek_transfer_ack(&expected(&mek), b"ACK"));
    // Another key of the same generation (a forged or wrong unwrap).
    let forged = MediaEncryptionKey::from_bytes([8; 32], 4);
    let forged_ack = encode_mek_transfer_ack(&transfer(), &forged, JOINER).unwrap();
    assert!(!is_mek_transfer_ack(&expected(&mek), &forged_ack));
    // Another party answering for the joiner.
    let other = encode_mek_transfer_ack(&transfer(), &mek, "cc").unwrap();
    assert!(!is_mek_transfer_ack(&expected(&mek), &other));
    // Every field the ack names is checked.
    for edit in [
        (|p: &mut MekTransferAckPayload| p.community_id = "c2".into())
            as fn(&mut MekTransferAckPayload),
        |p| p.requester_pseudonym = "cc".into(),
        |p| p.generation = 5,
        |p| p.channel_id = Some("ch".into()),
        |p| p.ack_tag.clear(),
        |p| p.transfer_id = vec![2; 16],
    ] {
        assert!(!is_mek_transfer_ack(&expected(&mek), &reencode(&ack, edit)));
    }
}
