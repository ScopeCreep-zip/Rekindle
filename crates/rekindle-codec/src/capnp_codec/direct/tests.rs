//! Every body round-trips, and the handshake frames fit Veilid's message
//! cap with the `'D'` tag at their worst case (plan C7.25's sizes: a
//! 4 KiB route blob, display name and message at their bounds in 4-byte
//! characters and a full PQXDH init). No frame carries a bundle since
//! plan E2.4.

use super::frame::tests::init;
use super::*;
use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

/// ML-KEM-768 ciphertext.
pub(super) const ML_KEM_CIPHERTEXT_LEN: usize = 1_088;
/// 3.5× the largest 1-hop route blob measured (1,178 B).
const ROUTE_BLOB_LEN: usize = 4_096;
/// The AEAD tag on a ratchet or HPKE ciphertext.
const AEAD_TAG_LEN: usize = 16;

fn s(text: &str) -> String {
    text.to_string()
}

fn widest(chars: usize) -> String {
    "😀".repeat(chars)
}

fn dht_key() -> String {
    format!("VLD0:{}", "A".repeat(43))
}

/// One body of every kind that has one.
fn samples() -> Vec<DirectBody> {
    let id = [0xA1; 16];
    vec![
        DirectBody::FriendRequest(FriendRequest {
            display_name: s("Alice"),
            message: s("hi"),
            profile_dht_key: dht_key(),
            route_blob: vec![9; 64],
            mailbox_dht_key: dht_key(),
            invite_id: Some(s("inv_01")),
        }),
        DirectBody::RelayEnvelope(RelayEnvelope {
            target_pubkey: "ab".repeat(32),
            inner_payload: vec![1, 2, 3],
        }),
        DirectBody::SessionResetRequest(SessionResetRequest {
            reason: s("decrypt failures"),
        }),
        DirectBody::SessionResetAccept,
        DirectBody::SessionResetDecline(SessionResetDecline { reason: s("no") }),
        DirectBody::DmText(DmText {
            client_id: id,
            body: s("hello"),
            reply_to: Some([0xB2; 16]),
            expire_seconds: 86_400,
        }),
        DirectBody::FriendAccept(FriendAccept {
            profile_dht_key: dht_key(),
            route_blob: vec![8; 32],
            mailbox_dht_key: dht_key(),
        }),
        DirectBody::FriendReject,
        DirectBody::Unfriended,
        DirectBody::UnfriendedAck,
        DirectBody::ProfileKeyRotated(ProfileKeyRotated {
            new_profile_dht_key: dht_key(),
        }),
        DirectBody::IdentityRotated(IdentityRotated {
            new_identity: [3; 32],
            sig_old: [4; 64],
            sig_new: [5; 64],
        }),
        DirectBody::DmEdit(DmEdit {
            target_client_id: id,
            body: s("hello!"),
            edited_at: 1_700_000_000_000,
        }),
        DirectBody::DmDelete(DmDelete {
            target_client_id: id,
        }),
        DirectBody::DmReaction(DmReaction {
            target_client_id: id,
            emoji: s("👍"),
            add: true,
        }),
        DirectBody::GroupDmInvite(GroupDmInvite {
            record_key: dht_key(),
            slot_seed: vec![0x5E; 32],
            initiator_pseudonym: "ab".repeat(32),
            participants_json: s(r#"[{"pseudonym":"p0","subkey":0,"publicKey":"pk0"}]"#),
            wrapped_mek: vec![0x3C; 92],
            mek_generation: 2,
        }),
        DirectBody::GroupDmLeave(GroupDmLeave {
            record_key: dht_key(),
        }),
        DirectBody::RelayOffer(RelayOffer {
            relay_route_blob: vec![6; 40],
            relay_pseudonym: "cd".repeat(32),
        }),
        DirectBody::RelayWithdraw(RelayWithdraw {
            relay_pseudonym: "cd".repeat(32),
        }),
        DirectBody::RelayOfferAck(RelayOfferAck {
            ok: false,
            reason: s("pool full"),
        }),
        DirectBody::StatusRequest(StatusRequest {
            target_pubkey: "ef".repeat(32),
        }),
        DirectBody::StatusResponse(StatusResponse {
            target_pubkey: "ef".repeat(32),
            status: s("away"),
            status_message: Some(s("brb")),
            last_seen: 1_700_000_000,
            route_blob: vec![7; 20],
        }),
        DirectBody::CallInvite(CallInvite {
            call_id: "0a".repeat(16),
            offer_kind: 1,
            initiator_pubkey: "ab".repeat(32),
            initiator_x25519_pub: vec![2; 32],
            expires_at_ms: 30_000,
            video_decode_codecs: vec![s("vp9"), s("h264")],
        }),
        DirectBody::CallRinging(CallRinging {
            call_id: "0a".repeat(16),
        }),
        DirectBody::CallAccept(CallAccept {
            call_id: "0a".repeat(16),
            acceptor_x25519_pub: vec![3; 32],
            video_decode_codecs: vec![s("vp8")],
        }),
        DirectBody::CallDecline(CallClosed {
            call_id: "0a".repeat(16),
            reason: s("busy"),
        }),
        DirectBody::CallEnd(CallClosed {
            call_id: "0a".repeat(16),
            reason: String::new(),
        }),
        DirectBody::CallMediaState(CallMediaState {
            call_id: "0a".repeat(16),
            audio: true,
            video: false,
            screen: true,
            timestamp_ms: 5,
        }),
        DirectBody::CallReaction(CallReaction {
            call_id: "0a".repeat(16),
            emoji: s("❤️"),
            timestamp_ms: 6,
        }),
        DirectBody::GroupCallOffer(GroupCallOffer {
            call_id: "0b".repeat(16),
            offer_kind: 0,
            initiator_pubkey: "ab".repeat(32),
            initiator_x25519_pub: vec![4; 32],
            participants: vec!["cd".repeat(32), "ef".repeat(32)],
            wrapped_call_key: vec![5; 60],
            expires_at_ms: 60_000,
        }),
        DirectBody::GroupCallAccept(GroupCallMember {
            call_id: "0b".repeat(16),
            pubkey: "cd".repeat(32),
        }),
        DirectBody::GroupCallDecline(CallClosed {
            call_id: "0b".repeat(16),
            reason: s("no"),
        }),
        DirectBody::GroupCallParticipantJoined(GroupCallMember {
            call_id: "0b".repeat(16),
            pubkey: "ef".repeat(32),
        }),
        DirectBody::GroupCallParticipantLeft(GroupCallParticipantLeft {
            call_id: "0b".repeat(16),
            participant_pubkey: "ef".repeat(32),
            reason: s("left"),
        }),
        DirectBody::DmVideoFragment(DmVideoFragment {
            stream_id: [6; 16],
            frame_seq: 9,
            fragment_index: 1,
            fragment_count: 3,
            keyframe: true,
            codec: s("vp9"),
            timestamp: 33,
            chunk: vec![7; 900],
        }),
        DirectBody::RegisterPushRelay(RegisterPushRelay {
            device_push_token: s("tok"),
            platform: s("fcm"),
            record_keys: vec![dht_key(), dht_key()],
        }),
        DirectBody::UnregisterPushRelay(UnregisterPushRelay {
            device_push_token: s("tok"),
        }),
        DirectBody::WakeNotify(WakeNotify { ts: 1_700_000_000 }),
        DirectBody::Typing(Typing { typing: true }),
    ]
}

/// The kinds §6 allocates whose bodies a later step defines, and the
/// PreKeyInit, which is a frame.
const NO_BODY_YET: [DmTypeId; 4] = [
    DmTypeId::PreKeyInit,
    DmTypeId::RetryRequest,
    DmTypeId::GroupDmAccept,
    DmTypeId::GroupDmDecline,
];

#[test]
fn every_body_round_trips_under_its_kind() {
    let samples = samples();
    for body in &samples {
        let bytes = body.encode();
        assert_eq!(
            DirectBody::decode(body.type_id(), &bytes).as_ref(),
            Ok(body),
            "{:?}",
            body.type_id()
        );
    }
    // Every allocated kind is either sampled or waits for its step.
    for kind in DmTypeId::ALL {
        let sampled = samples.iter().any(|b| b.type_id() == *kind);
        assert_ne!(sampled, NO_BODY_YET.contains(kind), "{kind:?}");
    }
    for kind in NO_BODY_YET {
        assert!(matches!(
            DirectBody::decode(kind, &[]),
            Err(CodecError::UnknownVariant(_))
        ));
    }
}

#[test]
fn absent_optionals_round_trip_and_bad_lengths_are_refused() {
    let text = DirectBody::DmText(DmText {
        client_id: [1; 16],
        body: String::new(),
        reply_to: None,
        expire_seconds: 0,
    });
    assert_eq!(
        DirectBody::decode(DmTypeId::DmText, &text.encode()),
        Ok(text)
    );
    let request = DirectBody::FriendRequest(FriendRequest {
        display_name: s("A"),
        message: String::new(),
        profile_dht_key: dht_key(),
        route_blob: Vec::new(),
        mailbox_dht_key: dht_key(),
        invite_id: None,
    });
    assert_eq!(
        DirectBody::decode(DmTypeId::FriendRequest, &request.encode()),
        Ok(request)
    );

    // A 15-byte client id is not a UUID.
    let bytes = encode_root::<crate::direct_capnp::dm_delete::Owned>(|mut b| {
        b.set_target_client_id(&[1; 15]);
    });
    assert!(DirectBody::decode(DmTypeId::DmDelete, &bytes).is_err());
    // An empty-bodied kind with bytes, and bytes of another kind.
    assert!(DirectBody::decode(DmTypeId::FriendReject, &[0]).is_err());
    assert!(DirectBody::decode(DmTypeId::DmText, &[0xFF; 3]).is_err());
}

/// The `'D'` datagram length of `frame`.
fn datagram_len(frame: &DmFrame) -> usize {
    1 + frame.encode().len()
}

/// A PreKeyInit whose first payload is `body`, at full PQXDH size.
fn pre_key_init(body: &DirectBody) -> DmFrame {
    let content = DmContent {
        content_hint: ContentHint::Resendable,
        sent_at_ms: u64::MAX,
        payload: body.encode(),
    };
    DmFrame {
        type_id: DmTypeId::PreKeyInit,
        sender: [0x77; 32],
        body: FrameBody::PreKeyInit(init(body.type_id(), content.encode().len() + AEAD_TAG_LEN)),
    }
}

/// A sealed frame of `body`.
fn sealed(body: &DirectBody) -> DmFrame {
    let content = SealedContent {
        nonce: [0x5A; 16],
        sent_at_ms: u64::MAX,
        payload: body.encode(),
    };
    DmFrame {
        type_id: body.type_id(),
        sender: [0x77; 32],
        body: FrameBody::Sealed(SealedBody {
            enc: [0x44; 32],
            ciphertext: vec![0x5A; content.encode().len() + AEAD_TAG_LEN],
            signature: [0x66; 64],
        }),
    }
}

#[test]
fn a_pre_key_init_carrying_friend_accept_fits() {
    let frame = pre_key_init(&DirectBody::FriendAccept(FriendAccept {
        profile_dht_key: dht_key(),
        route_blob: vec![0xAB; ROUTE_BLOB_LEN],
        mailbox_dht_key: dht_key(),
    }));
    assert_eq!(DmFrame::decode(&frame.encode()).as_ref(), Ok(&frame));
    let len = datagram_len(&frame);
    assert!(
        len <= MAX_APP_MESSAGE_LEN,
        "PreKeyInit(FriendAccept) {len} B"
    );
}

#[test]
fn a_pre_key_init_carrying_session_reset_accept_fits() {
    let len = datagram_len(&pre_key_init(&DirectBody::SessionResetAccept));
    assert!(
        len <= MAX_APP_MESSAGE_LEN,
        "PreKeyInit(SessionResetAccept) {len} B"
    );
}

#[test]
fn every_sealed_handshake_frame_fits() {
    let frames = [
        sealed(&DirectBody::FriendRequest(FriendRequest {
            display_name: widest(rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN),
            message: widest(rekindle_types::key_format::MAX_MESSAGE_LEN),
            profile_dht_key: dht_key(),
            route_blob: vec![0xAB; ROUTE_BLOB_LEN],
            mailbox_dht_key: dht_key(),
            invite_id: Some("0f".repeat(18)),
        })),
        sealed(&DirectBody::SessionResetRequest(SessionResetRequest {
            reason: widest(256),
        })),
        sealed(&DirectBody::SessionResetDecline(SessionResetDecline {
            reason: widest(256),
        })),
        sealed(&DirectBody::FriendReject),
    ];
    for frame in frames {
        assert_eq!(DmFrame::decode(&frame.encode()).as_ref(), Ok(&frame));
        let len = datagram_len(&frame);
        assert!(len <= MAX_APP_MESSAGE_LEN, "{:?} {len} B", frame.type_id);
    }
}
