//! The two admissions of a friend request from a stranger (plan E2.5,
//! D17): a friend-code token compared in constant time, a stale or wrong
//! one dropped with the same empty receipt as an admitted one; and an
//! invite, single-use, refused once cancelled, expired or answered by
//! someone else.

use rekindle_codec::capnp_codec::direct::RequestAuth;

use super::direct_tests::{
    ann, call, now_ms, received_events, requests, sealed_request_with, ANN_TOKEN,
};
use super::Host;

/// A request from a new stranger to Ann admitted by `auth`; its reply.
async fn request(host: &Host, owner: [u8; 32], nonce: u8, auth: RequestAuth) -> Vec<u8> {
    let stranger = rekindle_crypto::Identity::generate();
    call(
        host,
        &sealed_request_with(&stranger, owner, [nonce; 16], now_ms(), auth),
    )
    .await
}

/// Track an invite of Ann's, made at `created_at`.
async fn invite(host: &Host, owner: [u8; 32], id: &'static str, created_at: i64) {
    let owner = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::outgoing_invites::insert(
                conn,
                &owner,
                id,
                "rekindle://x",
                created_at,
            )
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_stale_token_is_dropped_silently_and_the_current_one_admitted() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);

    let stale = RequestAuth::Token(format!("{}e", "a".repeat(25)));
    assert!(
        request(&host, owner, 1, stale).await.is_empty(),
        "no distinguishing reply"
    );
    assert_eq!(requests(&host, owner).await, 0, "nothing stored");
    assert_eq!(received_events(&mut events).await, 0, "no event");

    let current = RequestAuth::Token(ANN_TOKEN.into());
    assert!(request(&host, owner, 2, current).await.is_empty());
    assert_eq!(requests(&host, owner).await, 1);
    assert_eq!(received_events(&mut events).await, 1);
}

#[tokio::test]
async fn an_invite_admits_one_requester_and_never_after_it_expires() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let now = i64::try_from(now_ms()).unwrap();
    invite(&host, owner, "open", now).await;
    invite(
        &host,
        owner,
        "old",
        now - rekindle_db::repo::outgoing_invites::INVITE_EXPIRY_MS - 1,
    )
    .await;

    request(&host, owner, 1, RequestAuth::Invite("open".into())).await;
    assert_eq!(
        requests(&host, owner).await,
        1,
        "the first requester is admitted"
    );
    request(&host, owner, 2, RequestAuth::Invite("open".into())).await;
    assert_eq!(
        requests(&host, owner).await,
        1,
        "a used invite admits no one else"
    );
    request(&host, owner, 3, RequestAuth::Invite("old".into())).await;
    request(&host, owner, 4, RequestAuth::Invite("unknown".into())).await;
    assert_eq!(
        requests(&host, owner).await,
        1,
        "expired and unknown invites are refused"
    );
}
