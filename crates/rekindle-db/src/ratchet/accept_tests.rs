//! `accept_init` under the prekey record (plan E2.4): B8's one
//! transaction (consume, record a last-resort use, store the session,
//! refill the slot) only after the first message decrypts, B9's replay
//! drop, and B5's unknown prekeys once the archive has swept them.

use rekindle_codec::capnp_codec::identity::decode_opk_slot;
use rekindle_crypto::signal::prekey_record::{plan_maintenance, RETIRED_ARCHIVE_MS};
use rekindle_crypto::signal::{PqPrekeyId, PreKeyInit};
use rekindle_types::dht_layout::prekeys::FIRST_SLOT;

use super::tests::{initiate, is_duplicate, AD_EXTRA};
use super::*;
use crate::prekeys::{self, apply_maintenance, local_prekeys, published_values, sweep_archive};
use crate::test_support::Party;

async fn accept(bob: &Party, alice: &Party, init: PreKeyInit) -> Result<Accepted, RatchetError> {
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
}

/// Alice initiates to Bob on his head alone (no slot).
async fn initiate_last_resort(alice: &Party, bob: &Party) -> PreKeyInit {
    let bundle = bob.bundle(None).await;
    initiate_and_record_outbound(
        &alice.db,
        &alice.local,
        &bob.address(),
        bundle,
        b"lr",
        AD_EXTRA,
    )
    .await
    .unwrap()
}

/// The accept consumes the slot's keys, refills the slot in the same
/// transaction, and hands back the new slot for the record; the record
/// as storage now describes it holds the refill.
#[tokio::test]
async fn an_accepted_init_consumes_and_refills_its_slot_at_once() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"first").await;
    let used = (init.one_time_prekey_id.unwrap(), init.pq_prekey);

    let accepted = accept(&bob, &alice, init).await.unwrap();
    assert_eq!(accepted.inbound.plaintext, b"first");
    assert_eq!(accepted.refills.len(), 1);
    let refill = &accepted.refills[0];
    assert_eq!(refill.subkey, FIRST_SLOT);
    let slot = decode_opk_slot(&refill.value).unwrap();
    assert_ne!(slot.opk_id, used.0);
    assert_ne!(PqPrekeyId::OneTime(slot.pq_opk_id), used.1);

    let published = published_values(&bob.db, &bob.local).await.unwrap();
    assert!(
        published.contains(refill),
        "the reconciliation sees the refill"
    );
    let local = local_prekeys(&bob.db, &bob.local.owner()).await.unwrap();
    assert!(plan_maintenance(bob.local.now_ms, &local).is_empty());
    assert_eq!(
        bob.scalar(
            "SELECT count(*) FROM signal_prekeys WHERE consumed_at IS NOT NULL AND secret = x''"
        )
        .await,
        2
    );
    assert_eq!(bob.scalar("SELECT count(*) FROM prekey_lr_inits").await, 0);
}

/// A first message that does not decrypt stores nothing: no session, no
/// consumption, no refill, no last-resort record.
#[tokio::test]
async fn a_failed_first_message_consumes_and_refills_nothing() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    for last_resort in [false, true] {
        let mut init = if last_resort {
            initiate_last_resort(&alice, &bob).await
        } else {
            initiate(&alice, &bob, b"first").await
        };
        let rows = bob.count("signal_prekeys").await;
        init.message.ciphertext[0] ^= 1;
        let result = accept(&bob, &alice, init).await;
        assert!(matches!(result, Err(RatchetError::Crypto(_))), "{result:?}");
        assert_eq!(bob.count("signal_prekeys").await, rows);
        assert_eq!(
            bob.scalar("SELECT count(*) FROM signal_prekeys WHERE consumed_at IS NOT NULL")
                .await,
            0
        );
        assert_eq!(bob.count("ratchet_sessions").await, 0);
        assert_eq!(bob.count("prekey_lr_inits").await, 0);
        assert_eq!(bob.count("unprocessed").await, 0);
    }
}

/// A last-resort init is recorded (B8); the same init again while its
/// session exists is a duplicate (B4), and after the session is gone it
/// is a replay (B9). Neither changes anything.
#[tokio::test]
async fn a_last_resort_init_is_recorded_and_its_replay_dropped() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate_last_resort(&alice, &bob).await;
    assert_eq!(init.one_time_prekey_id, None);
    let accepted = accept(&bob, &alice, init.clone()).await.unwrap();
    assert!(accepted.refills.is_empty(), "no slot was used");
    assert_eq!(bob.count("prekey_lr_inits").await, 1);

    assert!(is_duplicate(&accept(&bob, &alice, init.clone()).await));

    forget(&bob.db, &bob.local.owner(), &alice.local.owner())
        .await
        .unwrap();
    let replay = accept(&bob, &alice, init).await;
    assert!(
        matches!(replay, Err(RatchetError::ReplayedLastResortInit)),
        "{replay:?}"
    );
    assert_eq!(bob.count("ratchet_sessions").await, 0);
    assert_eq!(bob.count("unprocessed").await, 1);

    // A fresh last-resort init (another base key) still opens.
    let again = initiate_last_resort(&alice, &bob).await;
    accept(&bob, &alice, again).await.unwrap();
    assert_eq!(bob.count("prekey_lr_inits").await, 2);
}

/// After a head rotation the old signed prekey and last-resort key still
/// open an init in flight; once the archive sweeps them the init names
/// an unknown prekey (B5), and B9's set goes with the last-resort key.
#[tokio::test]
async fn a_retired_head_opens_until_the_archive_sweeps_it() {
    let (alice, bob, carol) = (Party::new(1), Party::new(2), Party::new(3));
    let early = initiate_last_resort(&alice, &bob).await;
    let in_flight = initiate_last_resort(&carol, &bob).await;
    accept(&bob, &alice, early).await.unwrap();

    let publishes = apply_maintenance(
        &bob.db,
        &bob.local,
        vec![rekindle_crypto::signal::prekey_record::PrekeyAction::RotateHead],
    )
    .await
    .unwrap();
    assert_eq!(publishes.len(), 1);
    assert_eq!(
        bob.scalar("SELECT count(*) FROM signal_prekeys WHERE retired_at IS NOT NULL")
            .await,
        2
    );

    let mut later = bob.local.clone();
    later.now_ms += RETIRED_ARCHIVE_MS;
    let swept = sweep_archive(&bob.db, &later.owner(), later.now_ms)
        .await
        .unwrap();
    assert_eq!((swept.prekeys, swept.lr_inits), (2, 1));
    let result = accept_init(
        &bob.db,
        &later,
        &carol.address(),
        in_flight,
        Reinitiation::Refused,
        AD_EXTRA,
        1,
    )
    .await;
    assert!(
        matches!(
            result,
            Err(RatchetError::UnknownPrekey {
                kind: prekeys::PrekeyKind::Spk,
                ..
            })
        ),
        "{result:?}"
    );
}

/// An init held back across the rotation (before the sweep) still opens
/// on the retired head.
#[tokio::test]
async fn an_init_in_flight_opens_on_the_retired_head() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let in_flight = initiate(&alice, &bob, b"slow").await;
    apply_maintenance(
        &bob.db,
        &bob.local,
        vec![rekindle_crypto::signal::prekey_record::PrekeyAction::RotateHead],
    )
    .await
    .unwrap();
    let accepted = accept(&bob, &alice, in_flight).await.unwrap();
    assert_eq!(accepted.inbound.plaintext, b"slow");
    assert_eq!(accepted.refills.len(), 1);
}

/// The initiate records the slot as used against the peer, in its own
/// transaction (B2), and a head-only initiate records none.
#[tokio::test]
async fn the_initiate_records_the_slot_it_used() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"first").await;
    let used = prekeys::used_slots(&alice.db, &alice.local.owner(), &bob.address())
        .await
        .unwrap();
    assert_eq!(
        used.into_iter().collect::<Vec<_>>(),
        [init.one_time_prekey_id.unwrap()]
    );
    initiate_last_resort(&Party::new(3), &bob).await;
    assert_eq!(alice.count("prekey_slots_used").await, 1);
}

/// B6: a rejection naming our unanswered init discards it; one naming
/// another base key, or an init the peer already answered, changes
/// nothing.
#[tokio::test]
async fn a_rejection_discards_only_our_unanswered_init() {
    use rekindle_crypto::signal::prekey_record::base_key_hash;
    let (alice, bob) = (Party::new(1), Party::new(2));
    let init = initiate(&alice, &bob, b"first").await;
    let hash = base_key_hash(&init.base_key);
    let now = alice.local.now_ms;
    let owner = alice.local.owner();

    assert!(
        !discard_pending_init(&alice.db, &owner, &bob.address(), [7; 32], now)
            .await
            .unwrap()
    );
    assert!(
        discard_pending_init(&alice.db, &owner, &bob.address(), hash, now)
            .await
            .unwrap()
    );
    // Nothing is pending any more, so a second rejection changes nothing.
    assert!(
        !discard_pending_init(&alice.db, &owner, &bob.address(), hash, now)
            .await
            .unwrap()
    );
}

/// R2 / DR §8.1: a consumption's commit truncates the WAL, so the erased
/// one-time secrets and the replaced session are in no `-wal` frame (a
/// plain write leaves its frames there).
#[tokio::test]
async fn a_consumption_truncates_the_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bob.db");
    let wal = dir.path().join("bob.db-wal");
    let (alice, bob) = (Party::new(1), Party::on_disk(2, &path).await);
    let init = initiate(&alice, &bob, b"first").await;
    assert!(
        std::fs::metadata(&wal).unwrap().len() > 0,
        "storing the record wrote WAL frames"
    );
    accept(&bob, &alice, init).await.unwrap();
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
}

/// A reset's init opens a session only when it answers our stored
/// request, consumed in the accepting transaction (plan audit D-4); an
/// unrequested one is refused before any prekey is looked up.
#[tokio::test]
async fn a_reset_init_needs_our_stored_request() {
    let (alice, bob) = (Party::new(1), Party::new(2));
    let reset = |init| {
        accept_init(
            &bob.db,
            &bob.local,
            &alice.address(),
            init,
            Reinitiation::Requested,
            AD_EXTRA,
            1,
        )
    };
    let unrequested = reset(initiate(&alice, &bob, b"reset").await).await;
    assert!(
        matches!(unrequested, Err(RatchetError::UnrequestedReset)),
        "{unrequested:?}"
    );

    let (owner, peer) = (
        hex::encode(bob.local.owner()),
        hex::encode(alice.address().identity),
    );
    let (o, p) = (owner.clone(), peer.clone());
    bob.db
        .call(move |conn| crate::repo::session_reset_requests::insert(conn, &o, &p, 1))
        .await
        .unwrap();
    let accepted = reset(initiate(&alice, &bob, b"answered").await)
        .await
        .unwrap();
    assert_eq!(accepted.inbound.plaintext, b"answered");
    let left = bob
        .db
        .call(move |conn| crate::repo::session_reset_requests::take(conn, &owner, &peer))
        .await
        .unwrap();
    assert!(!left, "the request is consumed with the accept");
}
