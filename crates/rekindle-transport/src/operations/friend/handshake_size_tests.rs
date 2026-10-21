//! Every friend-handshake frame fits Veilid's message cap (plans C7.25,
//! E2.3).
//!
//! FriendAccept was ~37 KB and FriendRequest ~33 KB against Veilid's
//! 32,768-byte `app_message` limit while the prekey bundle travelled as
//! JSON inside the JSON payload, so every invite failed. No frame carries a
//! bundle since E2.4 (the initiator reads the prekey record). These seal
//! each handshake frame at its worst case, the PreKeyInit from a bundle
//! assembled from a real record minted by `rekindle_db::prekeys` over a
//! temporary database (a one-time slot included), and wire it as every 1:1
//! send does (`operations::dm::wire`, the `'D'` tag included).
//!
//! A `FriendRequest` goes by `app_call`, whose cap is the same 32,768 B
//! (`~/veilid/veilid-core/src/rpc_processor/coders/operations/operation_app_call.rs:3-4`):
//! it is measured HPKE-sealed with each admission it can carry, the friend
//! code's token or an invite (plan E2.5, D17).

use rekindle_codec::capnp_codec::direct::{
    DirectBody, DmContent, DmFrame, DmTypeId, FrameBody, FriendAccept, FriendRequest, PqPrekey,
    PreKeyRejected, RegisterPushRelay, RequestAuth, SessionResetDecline, SessionResetRequest,
    UnregisterPushRelay, WakeNotify,
};
use rekindle_db::test_support::Party;
use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

use crate::operations::dm::frame::{ad_extra, init_body, seal_presession};
use crate::operations::dm::wire;

/// 3.5× the largest 1-hop route blob measured (1,178 B), room for 3-hop
/// inbound routes.
const ROUTE_BLOB_LEN: usize = 4_096;

/// A display name and message at their bounds, every character 4 bytes.
fn widest(chars: usize) -> String {
    "😀".repeat(chars)
}

fn dht_key() -> String {
    format!("VLD0:{}", "A".repeat(43))
}

/// The wire length of `body` sealed from `from` to `to`.
fn sealed_len(from: &Party, to: &Party, body: &DirectBody) -> usize {
    let frame = seal_presession(
        &from.local.identity,
        &to.local.owner(),
        body,
        [0xFF; 16],
        u64::MAX,
    )
    .unwrap();
    wire(&frame)
        .unwrap_or_else(|e| panic!("{:?} does not fit: {e}", body.type_id()))
        .len()
}

/// The wire length of the PreKeyInit from `from` that opens a session with
/// `to` from a one-time slot of its record, carrying `first`.
async fn init_len(from: &Party, to: &Party, first: &DirectBody) -> usize {
    let bundle = to
        .bundle(Some(rekindle_types::dht_layout::prekeys::FIRST_SLOT))
        .await;
    assert!(bundle.one_time_prekey.is_some() && bundle.pqpk_ot.is_some());
    let content = DmContent {
        stream_seq: 0,
        content_hint: rekindle_codec::capnp_codec::direct::ContentHint::Default,
        sent_at_ms: u64::MAX,
        payload: first.encode(),
    }
    .encode();
    let init = rekindle_db::ratchet::initiate_and_record_outbound(
        &from.db,
        &from.local,
        &to.address(),
        bundle,
        &content,
        &ad_extra(first.type_id()),
    )
    .await
    .unwrap();
    let frame = DmFrame {
        type_id: DmTypeId::PreKeyInit,
        sender: from.local.owner(),
        body: FrameBody::PreKeyInit(init_body(init, first.type_id())),
    };
    wire(&frame)
        .unwrap_or_else(|e| {
            panic!(
                "PreKeyInit carrying {:?} does not fit: {e}",
                first.type_id()
            )
        })
        .len()
}

#[tokio::test]
async fn friend_request_fits_by_app_call_with_either_admission() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let token = rekindle_secrets::friend_token::mint().as_str().to_string();
    let invite = uuid::Uuid::new_v4().to_string();
    for auth in [RequestAuth::Token(token), RequestAuth::Invite(invite)] {
        let request = DirectBody::FriendRequest(FriendRequest {
            display_name: widest(rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN),
            message: widest(rekindle_types::key_format::MAX_MESSAGE_LEN),
            profile_dht_key: dht_key(),
            route_blob: vec![0xAB; ROUTE_BLOB_LEN],
            mailbox_dht_key: dht_key(),
            auth: auth.clone(),
        });
        let len = sealed_len(&alice, &bob, &request);
        assert!(len <= MAX_APP_MESSAGE_LEN, "FriendRequest {auth:?} {len} B");
    }
}

/// The friend code and the invite URL each come back out of the OS
/// deep-link parser as what was printed.
#[test]
fn the_friend_code_and_the_invite_url_round_trip_through_deep_link_parse() {
    use rekindle_types::friend_code::{FriendCode, MAX_FRIEND_CODE_URL_LEN};
    use rekindle_types::invite::{DeepLink, MAX_FRIEND_INVITE_URL_LEN};

    let secret = [7u8; 32];
    let public_key = hex::encode(
        rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&secret)
            .verifying_key()
            .to_bytes(),
    );
    let token = rekindle_secrets::friend_token::mint();
    let url = format!(
        "rekindle://add/{public_key}/{}/{}",
        dht_key(),
        token.as_str()
    );
    let code = FriendCode::parse(&url).unwrap();
    assert!(code.to_url().len() <= MAX_FRIEND_CODE_URL_LEN);
    assert_eq!(
        DeepLink::parse(&code.to_url()),
        Ok(DeepLink::Add(code.clone()))
    );

    let blob = rekindle_codec::message::create_invite_blob(
        &secret,
        &public_key,
        &widest(rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN),
        &dht_key(),
        &dht_key(),
        &[0xAB; ROUTE_BLOB_LEN],
        Some(&uuid::Uuid::new_v4().to_string()),
        1_715_000_000_000,
    );
    let invite = rekindle_codec::message::encode_invite_url(&blob);
    assert!(
        invite.len() <= MAX_FRIEND_INVITE_URL_LEN,
        "{} B",
        invite.len()
    );
    let Ok(DeepLink::FriendInvite(parsed)) = DeepLink::parse(&invite) else {
        panic!("the invite URL is not a friend invite");
    };
    let decoded = rekindle_codec::message::decode_invite_url(&parsed).unwrap();
    rekindle_codec::message::verify_invite_blob(&decoded).unwrap();
    assert_eq!(decoded.invite_id, blob.invite_id);
    assert_eq!(decoded.route_blob, blob.route_blob);
}

#[tokio::test]
async fn friend_accept_fits() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let accept = DirectBody::FriendAccept(FriendAccept {
        profile_dht_key: dht_key(),
        route_blob: vec![0xAB; ROUTE_BLOB_LEN],
        mailbox_dht_key: dht_key(),
    });
    let len = init_len(&bob, &alice, &accept).await;
    assert!(
        len <= MAX_APP_MESSAGE_LEN,
        "PreKeyInit + FriendAccept {len} B"
    );
}

#[tokio::test]
async fn every_presession_frame_and_the_reset_accept_fit() {
    // The push-relay kinds are sealed too (§6): a token and 64 record keys.
    let (alice, bob) = (Party::new(1), Party::new(2));
    let request = DirectBody::SessionResetRequest(SessionResetRequest {
        reason: widest(rekindle_types::key_format::MAX_NOTE_LEN),
    });
    let decline = DirectBody::SessionResetDecline(SessionResetDecline {
        reason: widest(rekindle_types::key_format::MAX_NOTE_LEN),
    });
    let register = DirectBody::RegisterPushRelay(RegisterPushRelay {
        device_push_token: "a".repeat(4_096),
        platform: "fcm".into(),
        record_keys: (0..64).map(|_| dht_key()).collect(),
    });
    let unregister = DirectBody::UnregisterPushRelay(UnregisterPushRelay {
        device_push_token: "a".repeat(4_096),
    });
    let wake = DirectBody::WakeNotify(WakeNotify { ts: u64::MAX });
    let rejected = DirectBody::PreKeyRejected(PreKeyRejected {
        signed_prekey_id: u32::MAX,
        one_time_prekey_id: Some(u32::MAX),
        pq_prekey: PqPrekey::OneTime(u32::MAX),
        base_key_hash: [0xFF; 32],
    });
    for body in [
        &request,
        &decline,
        &DirectBody::FriendReject,
        &register,
        &unregister,
        &wake,
        &rejected,
    ] {
        let len = sealed_len(&alice, &bob, body);
        assert!(len <= MAX_APP_MESSAGE_LEN, "{:?} {len} B", body.type_id());
    }
    let accept = init_len(&bob, &alice, &DirectBody::SessionResetAccept).await;
    assert!(
        accept <= MAX_APP_MESSAGE_LEN,
        "PreKeyInit + SessionResetAccept {accept} B"
    );
}
