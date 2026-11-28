//! Simultaneous initiation (glare, Sesame §3): two parties who each open a
//! session to the other converge on one working session on both sides,
//! by the crossing tiebreak (the higher base key, plan E2.5h).

use rekindle_crypto::signal::{Outgoing, PreKeyInit};
use rekindle_types::dht_layout::prekeys::FIRST_SLOT;

use super::tests::{initiate, receive, send, AD_EXTRA};
use super::*;
use crate::test_support::Party;

async fn accept(at: &Party, from: &Party, init: PreKeyInit) -> Result<Accepted, RatchetError> {
    accept_init(
        &at.db,
        &at.local,
        &from.address(),
        init,
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await
}

/// Both initiate; the higher init reaches the lower side first, which takes
/// it active and answers on it, and that answer reaches the higher side
/// before the lower side's own init. The late init is the losing side of
/// the crossing: kept archived with its first message delivered, never
/// refused, and both sides then talk on the higher session.
#[tokio::test]
async fn a_crossing_init_that_arrives_after_the_answer_is_kept_and_both_converge() {
    let (ann, bea) = (Party::new(1), Party::new(2));
    let from_ann = initiate(&ann, &bea, b"from ann").await;
    let from_bea = initiate(&bea, &ann, b"from bea").await;
    let (high, low, high_init, low_init) = if from_ann.base_key > from_bea.base_key {
        (&ann, &bea, from_ann, from_bea)
    } else {
        (&bea, &ann, from_bea, from_ann)
    };

    accept(low, high, high_init).await.unwrap();
    let answer = send(low, high, b"answer").await;
    assert_eq!(
        receive(high, low, answer).await.unwrap().plaintext,
        b"answer"
    );

    let late = accept(high, low, low_init).await.unwrap();
    assert!(
        late.inbound.plaintext == b"from ann" || late.inbound.plaintext == b"from bea",
        "the late init's first message is delivered"
    );
    assert_ne!(late.inbound.plaintext, b"answer");

    for round in 0..3u8 {
        let to_low =
            encrypt_and_record_outbound(&high.db, &high.local, &low.address(), &[round], AD_EXTRA)
                .await
                .unwrap();
        assert!(
            matches!(to_low, Outgoing::Message(_)),
            "high sends on an acknowledged session"
        );
        receive(low, high, to_low.message().clone()).await.unwrap();
        let to_high =
            encrypt_and_record_outbound(&low.db, &low.local, &high.address(), &[round], AD_EXTRA)
                .await
                .unwrap();
        assert!(
            matches!(to_high, Outgoing::Message(_)),
            "low sends on an acknowledged session"
        );
        receive(high, low, to_high.message().clone()).await.unwrap();
    }
}

/// A send found no session and fetched the peer's bundle; meanwhile the
/// peer's init opened one. The send answers on that session instead of
/// replacing it with an initiation the peer would refuse (Sesame §3.3).
#[tokio::test]
async fn opening_a_session_answers_one_the_peer_opened_meanwhile() {
    let (ann, bea) = (Party::new(1), Party::new(2));
    let fetched = ann.bundle(Some(FIRST_SLOT)).await;
    let init = initiate(&ann, &bea, b"hello").await;
    accept(&bea, &ann, init).await.unwrap();

    let out = open_and_record_outbound(
        &bea.db,
        &bea.local,
        &ann.address(),
        fetched,
        b"hi",
        AD_EXTRA,
    )
    .await
    .unwrap();
    assert!(
        matches!(out, Outgoing::Message(_)),
        "the open session is answered on"
    );
    assert_eq!(
        receive(&ann, &bea, out.message().clone())
            .await
            .unwrap()
            .plaintext,
        b"hi"
    );

    let cat = Party::new(3);
    let fresh = ann.bundle(Some(FIRST_SLOT)).await;
    let out =
        open_and_record_outbound(&cat.db, &cat.local, &ann.address(), fresh, b"new", AD_EXTRA)
            .await
            .unwrap();
    assert!(
        matches!(out, Outgoing::PreKey(_)),
        "with no session, the open initiates"
    );
}
