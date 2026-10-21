//! The ratchet's storage guarantees: committed before any send, one load
//! per peer, duplicates dropped, identity == address, and one-time keys
//! consumed only after the first decrypt (the B8/B9 cases are in
//! `accept_tests`).

use std::sync::Arc;

use rekindle_crypto::signal::{PreKeyInit, RatchetMessage};
use rekindle_crypto::{CryptoError, Identity};
use rekindle_types::dht_layout::prekeys::FIRST_SLOT;
use tokio::task::JoinSet;

use super::*;
use crate::repo::unprocessed::take_for_replay;
use crate::test_support::{Party, Seeded};

pub(super) const AD_EXTRA: &[u8] = b"typeId";
const DM: u16 = 0x0100;

/// Alice starts a session with Bob from his record's first slot.
pub(super) async fn initiate(alice: &Party, bob: &Party, first: &[u8]) -> PreKeyInit {
    let bundle = bob.bundle(Some(FIRST_SLOT)).await;
    initiate_and_record_outbound(
        &alice.db,
        &alice.local,
        &bob.address(),
        bundle,
        first,
        AD_EXTRA,
    )
    .await
    .unwrap()
}

async fn send(from: &Party, to: &Party, plaintext: &[u8]) -> RatchetMessage {
    encrypt_and_record_outbound(&from.db, &from.local, &to.address(), plaintext, AD_EXTRA)
        .await
        .unwrap()
}

async fn receive(
    at: &Party,
    from: &Party,
    message: RatchetMessage,
) -> Result<Inbound, RatchetError> {
    decrypt_and_record_inbound(&at.db, &at.local, &from.address(), message, AD_EXTRA, DM).await
}

async fn record_bytes(party: &Party, peer: &Party) -> Vec<u8> {
    let (owner, peer) = (party.local.owner(), peer.address());
    party
        .db
        .call(move |c| {
            c.query_row(
                "SELECT record FROM ratchet_sessions WHERE owner_key = ?1 AND peer_identity = ?2",
                params![identity_text(&owner), identity_text(&peer.identity)],
                |r| r.get(0),
            )
        })
        .await
        .unwrap()
}

pub(super) fn is_duplicate<T>(result: &Result<T, RatchetError>) -> bool {
    matches!(
        result,
        Err(RatchetError::Crypto(CryptoError::DuplicateMessage))
    )
}

#[tokio::test]
async fn a_session_runs_both_ways_and_records_what_it_decrypts() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"hello").await;
    let first = accept_init(
        &bob.db,
        &bob.local,
        &alice.address(),
        init.clone(),
        Reinitiation::Refused,
        AD_EXTRA,
        0x0001,
    )
    .await
    .unwrap()
    .inbound;
    assert_eq!(first.plaintext, b"hello");
    assert_eq!(first.key.header_dh, init.message.header.dh);

    let reply = send(&bob, &alice, b"hi back").await;
    let got = receive(&alice, &bob, reply).await.unwrap();
    assert_eq!(got.plaintext, b"hi back");

    let owner = bob.local.owner();
    let replay = bob
        .db
        .call(move |c| take_for_replay(c, &owner, 2_000))
        .await
        .unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(
        (
            replay[0].key,
            replay[0].type_id,
            replay[0].plaintext.as_slice()
        ),
        (first.key, 0x0001, &b"hello"[..])
    );
    assert_eq!(bob.count("signal_identities").await, 1);
    assert_eq!(alice.count("signal_identities").await, 1);
}

#[tokio::test]
async fn forgetting_a_peer_ends_the_session() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"hello").await;
    accept_init(
        &bob.db,
        &bob.local,
        &alice.address(),
        init,
        Reinitiation::Refused,
        AD_EXTRA,
        0x0001,
    )
    .await
    .unwrap();
    forget(&bob.db, &bob.local.owner(), &alice.local.owner())
        .await
        .unwrap();
    assert_eq!(bob.count("ratchet_sessions").await, 0);
    let sent =
        encrypt_and_record_outbound(&bob.db, &bob.local, &alice.address(), b"x", AD_EXTRA).await;
    assert!(matches!(
        sent,
        Err(RatchetError::Crypto(CryptoError::NoSession(_)))
    ));
}

/// The encrypt commits the advanced record before the caller sends: a
/// crash between the two loses no key, the next message uses the next
/// key, and the retry of the unsent bytes still decrypts.
#[tokio::test]
async fn the_record_is_committed_before_any_send() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alice.db");
    let identity = Arc::new(Identity::generate());
    let open_alice = || {
        let db = crate::open(&path, &[3; 32]).unwrap();
        Party {
            db,
            local: LocalParty {
                identity: Arc::clone(&identity),
                rng: Arc::new(Seeded::new(1)),
                now_ms: 1_000,
            },
        }
    };
    let alice = open_alice();
    let owner = identity_text(&identity.public_key_bytes());
    alice
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO identity (public_key, created_at, friend_request_token) VALUES (?1, 0, 'aaaaaaaaaaaaaaaaaaaaaaaaaa')",
                [owner],
            )
        })
        .await
        .unwrap();
    let bob = Party::new(2);
    let init = initiate(&alice, &bob, b"first").await;
    accept_init(
        &bob.db,
        &bob.local,
        &alice.address(),
        init,
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await
    .unwrap();

    let unsent = send(&alice, &bob, b"lost in the crash").await;
    alice.db.close().await.unwrap();

    let alice = open_alice();
    let next = send(&alice, &bob, b"after restart").await;
    assert_eq!(next.header.dh, unsent.header.dh);
    assert_eq!(next.header.n, unsent.header.n + 1, "the key was not reused");
    assert_eq!(
        receive(&bob, &alice, next).await.unwrap().plaintext,
        b"after restart"
    );
    assert_eq!(
        receive(&bob, &alice, unsent.clone())
            .await
            .unwrap()
            .plaintext,
        b"lost in the crash",
        "the retry sends the recorded bytes"
    );
    assert!(is_duplicate(&receive(&bob, &alice, unsent).await));
}

/// Concurrent first messages for one peer (the same PreKeyInit delivered
/// N times at once) make one session; then N concurrent decrypts leave
/// exactly the record N sequential decrypts would.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_messages_one_session() {
    const N: usize = 8;
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"first").await;

    let mut accepts = JoinSet::new();
    for _ in 0..N {
        let (db, local, peer, init) = (
            bob.db.clone(),
            bob.local.clone(),
            alice.address(),
            init.clone(),
        );
        accepts.spawn(async move {
            accept_init(&db, &local, &peer, init, Reinitiation::Refused, AD_EXTRA, 1).await
        });
    }
    let results = accepts.join_all().await;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results.iter().filter(|r| r.is_err()).all(is_duplicate));
    assert_eq!(bob.count("unprocessed").await, 1);
    assert_eq!(bob.count("ratchet_sessions").await, 1);

    let mut messages = Vec::new();
    for i in 0..N {
        messages.push(send(&alice, &bob, format!("m{i}").as_bytes()).await);
    }
    let sequential = Party::new(3);
    let snapshot = record_bytes(&bob, &alice).await;
    let (owner, peer) = (sequential.local.owner(), alice.address());
    sequential
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO ratchet_sessions VALUES (?1, ?2, '', ?3, 0)",
                params![
                    identity_text(&owner),
                    identity_text(&peer.identity),
                    snapshot
                ],
            )
        })
        .await
        .unwrap();
    for message in &messages {
        receive(&sequential, &alice, message.clone()).await.unwrap();
    }

    let mut decrypts = JoinSet::new();
    for message in messages {
        let (db, local, peer) = (bob.db.clone(), bob.local.clone(), alice.address());
        decrypts.spawn(async move {
            decrypt_and_record_inbound(&db, &local, &peer, message, AD_EXTRA, DM).await
        });
    }
    for result in decrypts.join_all().await {
        result.unwrap();
    }
    assert_eq!(
        bob.count("unprocessed").await,
        1 + i64::try_from(N).unwrap()
    );
    assert_eq!(
        record_bytes(&bob, &alice).await,
        record_bytes(&sequential, &alice).await
    );
}

#[tokio::test]
async fn a_duplicate_is_dropped_and_changes_nothing() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"first").await;
    accept_init(
        &bob.db,
        &bob.local,
        &alice.address(),
        init.clone(),
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await
    .unwrap();
    let message = send(&alice, &bob, b"once").await;
    receive(&bob, &alice, message.clone()).await.unwrap();

    let before = record_bytes(&bob, &alice).await;
    assert!(is_duplicate(&receive(&bob, &alice, message).await));
    assert!(is_duplicate(
        &accept_init(
            &bob.db,
            &bob.local,
            &alice.address(),
            init,
            Reinitiation::Refused,
            AD_EXTRA,
            1
        )
        .await
    ));
    assert_eq!(record_bytes(&bob, &alice).await, before);
    assert_eq!(bob.count("unprocessed").await, 2);
}

/// An init from one identity delivered as another's is refused before
/// any prekey is touched, and nothing is stored.
#[tokio::test]
async fn an_identity_that_is_not_the_address_is_refused() {
    let (alice, bob, mallory) = (Party::new(1), Party::new(2), Party::new(3));
    let init = initiate(&mallory, &bob, b"i am alice").await;
    let result = accept_init(
        &bob.db,
        &bob.local,
        &alice.address(),
        init.clone(),
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await;
    assert!(
        matches!(
            result,
            Err(RatchetError::Crypto(
                CryptoError::IdentityAddressMismatch { .. }
            ))
        ),
        "{result:?}"
    );
    assert_eq!(bob.count("ratchet_sessions").await, 0);
    assert_eq!(bob.count("unprocessed").await, 0);
    assert_eq!(bob.count("signal_identities").await, 0);

    let bundle = bob.bundle(Some(FIRST_SLOT)).await;
    let result = initiate_and_record_outbound(
        &alice.db,
        &alice.local,
        &mallory.address(),
        bundle,
        b"x",
        AD_EXTRA,
    )
    .await;
    assert!(matches!(
        result,
        Err(RatchetError::Crypto(
            CryptoError::IdentityAddressMismatch { .. }
        ))
    ));
    assert_eq!(alice.count("ratchet_sessions").await, 0);

    // The genuine init still completes: nothing was consumed.
    accept_init(
        &bob.db,
        &bob.local,
        &mallory.address(),
        init,
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await
    .unwrap();
}

/// A first message that fails consumes no one-time key; the genuine one
/// consumes both (their slot is refilled), and the keys are gone for any
/// later init.
#[tokio::test]
async fn one_time_keys_are_consumed_only_after_the_first_decrypt() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"first").await;
    let unconsumed = "SELECT count(*) FROM signal_prekeys WHERE consumed_at IS NULL";
    let count = |party: &Party, sql: &'static str| {
        let db = party.db.clone();
        async move {
            db.call(move |c| c.query_row(sql, [], |r| r.get::<_, i64>(0)))
                .await
                .unwrap()
        }
    };
    // The head's two keys and a hundred slots of two.
    assert_eq!(count(&bob, unconsumed).await, 202);

    let mut tampered = init.clone();
    tampered.message.ciphertext[0] ^= 1;
    let result = accept_init(
        &bob.db,
        &bob.local,
        &alice.address(),
        tampered,
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await;
    assert!(matches!(result, Err(RatchetError::Crypto(_))), "{result:?}");
    assert_eq!(count(&bob, unconsumed).await, 202);
    assert_eq!(bob.count("ratchet_sessions").await, 0);

    accept_init(
        &bob.db,
        &bob.local,
        &alice.address(),
        init.clone(),
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        count(&bob, unconsumed).await,
        202,
        "two consumed, two minted into their slot"
    );
    let erased =
        "SELECT count(*) FROM signal_prekeys WHERE consumed_at IS NOT NULL AND secret = x''";
    assert_eq!(count(&bob, erased).await, 2);

    // Another peer naming the consumed key is refused.
    let carol = Party::new(4);
    let (_, mut reuse) = rekindle_crypto::signal::SessionRecord::new(bob.local.owner())
        .initiate(
            &carol.local.identity,
            &bob.bundle(Some(FIRST_SLOT)).await,
            b"x",
            AD_EXTRA,
            carol.local.rng.as_ref(),
        )
        .unwrap();
    reuse.one_time_prekey_id = init.one_time_prekey_id;
    let result = accept_init(
        &bob.db,
        &bob.local,
        &carol.address(),
        reuse,
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await;
    assert!(
        matches!(
            result,
            Err(RatchetError::UnknownPrekey {
                kind: PrekeyKind::Opk,
                ..
            })
        ),
        "{result:?}"
    );
}

#[tokio::test]
async fn sending_without_a_session_fails_and_stores_nothing() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let result =
        encrypt_and_record_outbound(&alice.db, &alice.local, &bob.address(), b"x", AD_EXTRA).await;
    assert!(matches!(
        result,
        Err(RatchetError::Crypto(CryptoError::NoSession(_)))
    ));
    assert_eq!(alice.count("ratchet_sessions").await, 0);
}

/// D24 send half (plan E2.5): a logged message is resent under the current
/// session on a request naming its key, and opens with its content; a
/// request naming another key resends nothing and moves no session.
#[tokio::test]
async fn a_logged_message_is_resent_only_on_its_ratchet_key() {
    use crate::repo::send_log;
    use rekindle_crypto::replay::ratchet_message_id;

    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"hello").await;
    accept_init(
        &bob.db,
        &bob.local,
        &alice.address(),
        init,
        Reinitiation::Refused,
        AD_EXTRA,
        0x0001,
    )
    .await
    .unwrap();
    let lost = encrypt_record_and_log_outbound(
        &alice.db,
        &alice.local,
        &bob.address(),
        b"the original",
        AD_EXTRA,
        DM,
    )
    .await
    .unwrap();
    let msg_id = ratchet_message_id(&lost.header.dh, lost.header.n);
    let (owner, peer) = (alice.local.owner(), bob.local.owner());

    let before = record_bytes(&alice, &bob).await;
    let forged = alice
        .db
        .call(move |c| send_log::take_for_resend(c, &owner, &peer, &msg_id, &[0xEE; 32], 5))
        .await
        .unwrap();
    assert_eq!(forged, None);
    assert_eq!(record_bytes(&alice, &bob).await, before);

    let observed = lost.header.dh;
    let logged = alice
        .db
        .call(move |c| send_log::take_for_resend(c, &owner, &peer, &msg_id, &observed, 5))
        .await
        .unwrap()
        .expect("the genuine request finds the row");
    assert_eq!((logged.type_id, logged.ratchet_key), (DM, observed));
    let resent = encrypt_record_and_log_outbound(
        &alice.db,
        &alice.local,
        &bob.address(),
        &logged.plaintext,
        AD_EXTRA,
        logged.type_id,
    )
    .await
    .unwrap();
    assert_ne!(
        (resent.header.dh, resent.header.n),
        (lost.header.dh, lost.header.n)
    );
    let got = receive(&bob, &alice, resent).await.unwrap();
    assert_eq!(got.plaintext, b"the original");
}
