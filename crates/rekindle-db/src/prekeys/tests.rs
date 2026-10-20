//! Our prekey rows under the record model (plan E2.4): a stored set is a
//! whole record, maintenance's actions mint and retire as B1 says, the
//! reconciliation's values are the published bytes, and the bookkeeping
//! tables keep B2's and B5's limits and the archive's horizons.

use rekindle_codec::capnp_codec::identity::decode_prekey_head;
use rekindle_crypto::signal::prekey_record::{
    mint_set, plan_maintenance, PrekeyAction, SlotState, OPK_STALE_MS, REJECTIONS_PER_WINDOW,
    REJECTION_WINDOW_MS, RETIRED_ARCHIVE_MS, SIGNED_ROTATE_AFTER_MS,
};
use rekindle_types::dht_layout::prekeys::{FIRST_SLOT, SLOT_COUNT};

use super::*;
use crate::ratchet::PeerAddress;
use crate::test_support::Party;

fn later(party: &Party, by_ms: i64) -> LocalParty {
    let mut local = party.local.clone();
    local.now_ms += by_ms;
    local
}

/// A stored set is a whole record: maintenance has nothing to do, and the
/// reconciliation's values are exactly what the set published.
#[tokio::test]
async fn a_stored_set_is_a_whole_record_and_reconciles_to_its_bytes() {
    let bob = Party::new(1);
    let set = mint_set(&bob.local.identity, bob.local.rng.as_ref(), 1_000);
    let expected = set_values(&set);
    let published = store_set(&bob.db, &bob.local.owner(), set).await.unwrap();
    assert_eq!(published, expected);
    assert_eq!(published.len(), 1 + 100);

    let local = local_prekeys(&bob.db, &bob.local.owner()).await.unwrap();
    assert!(local.head.is_some());
    assert_eq!(local.slots.len(), 100);
    assert!(plan_maintenance(1_000, &local).is_empty());
    assert_eq!(
        published_values(&bob.db, &bob.local).await.unwrap(),
        expected,
        "re-signing the rows reproduces the published bytes"
    );
}

/// An identity with no prekeys is minted whole by maintenance itself.
#[tokio::test]
async fn maintenance_mints_an_empty_record_whole() {
    let bob = Party::new(2);
    let local = local_prekeys(&bob.db, &bob.local.owner()).await.unwrap();
    let actions = plan_maintenance(bob.local.now_ms, &local);
    assert_eq!(actions.len(), 101);
    let published = apply_maintenance(&bob.db, &bob.local, actions)
        .await
        .unwrap();
    assert_eq!(published.len(), 101);
    assert_eq!(published[0].subkey, 0);
    let local = local_prekeys(&bob.db, &bob.local.owner()).await.unwrap();
    assert!(plan_maintenance(bob.local.now_ms, &local).is_empty());
    assert_eq!(
        published_values(&bob.db, &bob.local).await.unwrap(),
        published
    );
}

/// Two days on the head rotates: the old keys are retired (kept for the
/// archive), the new head carries new ids at the new time.
#[tokio::test]
async fn the_head_rotates_and_the_old_keys_are_retired() {
    let bob = Party::new(3);
    bob.record().await;
    let before = status(&bob.db, &bob.local.owner(), bob.local.now_ms)
        .await
        .unwrap()
        .unwrap();
    let at = later(&bob, SIGNED_ROTATE_AFTER_MS);
    let local = local_prekeys(&bob.db, &at.owner()).await.unwrap();
    let actions = plan_maintenance(at.now_ms, &local);
    assert_eq!(actions, [PrekeyAction::RotateHead]);
    let published = apply_maintenance(&bob.db, &at, actions).await.unwrap();
    let head = decode_prekey_head(&published[0].value).unwrap();
    assert_eq!(head.issued_at, at.now_ms);
    assert_ne!(head.spk_id, before.spk_id);
    assert_ne!(head.pq_lr_id, before.pq_lr_id);

    let after = status(&bob.db, &at.owner(), at.now_ms)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((after.spk_id, after.spk_age_ms), (head.spk_id, 0));
    assert_eq!(
        bob.scalar(
            "SELECT count(*) FROM signal_prekeys WHERE retired_at IS NOT NULL AND secret != x''"
        )
        .await,
        2,
        "the old secrets stay for the archive"
    );
}

/// A stale slot is replaced in place; a subkey outside the slots is
/// refused and nothing is stored.
#[tokio::test]
async fn a_stale_slot_is_replaced_and_a_bad_subkey_refused() {
    let bob = Party::new(4);
    bob.record().await;
    let at = later(&bob, OPK_STALE_MS);
    let rows = bob.count("signal_prekeys").await;
    let result = apply_maintenance(
        &bob.db,
        &at,
        vec![
            PrekeyAction::RotateHead,
            PrekeyAction::FillSlot { subkey: 0 },
        ],
    )
    .await;
    assert!(matches!(
        result,
        Err(RatchetError::InvalidSlot { subkey: 0 })
    ));
    assert_eq!(bob.count("signal_prekeys").await, rows, "nothing stored");

    let local = local_prekeys(&bob.db, &at.owner()).await.unwrap();
    let actions = plan_maintenance(at.now_ms, &local);
    assert_eq!(actions.len(), 1 + 100, "the head and every stale slot");
    apply_maintenance(&bob.db, &at, actions).await.unwrap();
    let local = local_prekeys(&bob.db, &at.owner()).await.unwrap();
    assert!(local.slots.values().all(|s| *s
        == SlotState::Live {
            created_at_ms: at.now_ms
        }));
}

/// The status read counts fresh slots and the consumed keys the archive
/// still holds.
#[tokio::test]
async fn the_status_counts_fresh_and_consumed_slots() {
    let bob = Party::new(5);
    assert_eq!(status(&bob.db, &bob.local.owner(), 0).await.unwrap(), None);
    bob.record().await;
    let owner = identity_text(&bob.local.owner());
    bob.db
        .call(move |c| {
            c.execute(
                "UPDATE signal_prekeys SET consumed_at = 1, secret = x'' \
                 WHERE owner_key = ?1 AND published_subkey IN (1, 2)",
                [owner],
            )
        })
        .await
        .unwrap();
    let row = status(&bob.db, &bob.local.owner(), 5_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((row.fresh_slots, row.consumed_slots), (98, 2));
    assert_eq!(row.spk_age_ms, 4_000);
    let local = local_prekeys(&bob.db, &bob.local.owner()).await.unwrap();
    assert_eq!(local.slots.get(&FIRST_SLOT), Some(&SlotState::Consumed));
    assert_eq!(
        plan_maintenance(5_000, &local),
        [
            PrekeyAction::FillSlot { subkey: 1 },
            PrekeyAction::FillSlot { subkey: 2 }
        ]
    );
}

/// The used slots are listed per peer device and swept once the peer's
/// record no longer lists them.
#[tokio::test]
async fn used_slots_are_listed_and_swept_per_device() {
    let alice = Party::new(6);
    let owner = alice.local.owner();
    let bob = PeerAddress::sole_device([7; 32]);
    let bob_phone = PeerAddress {
        identity: [7; 32],
        device: "phone".into(),
    };
    for (peer, id) in [(&bob, 11), (&bob, 12), (&bob_phone, 13)] {
        let (db, peer) = (alice.db.clone(), peer.clone());
        db.call(move |c| insert_slot_used(c, &owner, &peer, id, 1_000))
            .await
            .unwrap();
    }
    let ids = |peer: PeerAddress| {
        let db = alice.db.clone();
        async move { used_slots(&db, &owner, &peer).await.unwrap() }
    };
    assert_eq!(
        ids(bob.clone()).await.into_iter().collect::<Vec<_>>(),
        [11, 12]
    );
    assert_eq!(
        sweep_used_slots(&alice.db, &owner, &bob, vec![12, 99])
            .await
            .unwrap(),
        1
    );
    assert_eq!(ids(bob.clone()).await.into_iter().collect::<Vec<_>>(), [12]);
    assert_eq!(ids(bob_phone).await.into_iter().collect::<Vec<_>>(), [13]);
}

/// B5: one rejection per base key, and at most ten per sender per window.
#[tokio::test]
async fn rejections_are_limited_per_base_key_and_per_sender() {
    let bob = Party::new(8);
    let owner = bob.local.owner();
    let (mallory, carol) = ([1; 32], [2; 32]);
    let admit = |sender: [u8; 32], hash: u8, at: i64| {
        let db = bob.db.clone();
        async move {
            admit_rejection(&db, &owner, &sender, [hash; 32], at)
                .await
                .unwrap()
        }
    };
    assert_eq!(admit(mallory, 0, 1_000).await, RejectionVerdict::Send);
    assert_eq!(
        admit(mallory, 0, 1_001).await,
        RejectionVerdict::AlreadySent
    );
    for hash in 1..u8::try_from(REJECTIONS_PER_WINDOW).unwrap() {
        assert_eq!(admit(mallory, hash, 1_000).await, RejectionVerdict::Send);
    }
    assert_eq!(
        admit(mallory, 200, 1_000).await,
        RejectionVerdict::RateLimited
    );
    assert_eq!(admit(carol, 201, 1_000).await, RejectionVerdict::Send);
    assert_eq!(
        admit(mallory, 202, 1_000 + REJECTION_WINDOW_MS).await,
        RejectionVerdict::Send,
        "the window passed"
    );
}

/// The sweep deletes retired and consumed rows past the archive, the
/// rejections past it, and used slots past a slot's life; nothing newer.
#[tokio::test]
async fn the_archive_sweep_keeps_its_horizons() {
    let bob = Party::new(9);
    bob.record().await;
    let owner = bob.local.owner();
    apply_maintenance(&bob.db, &bob.local, vec![PrekeyAction::RotateHead])
        .await
        .unwrap();
    admit_rejection(&bob.db, &owner, &[1; 32], [1; 32], bob.local.now_ms)
        .await
        .unwrap();
    let peer = PeerAddress::sole_device([3; 32]);
    let now = bob.local.now_ms;
    bob.db
        .call(move |c| insert_slot_used(c, &owner, &peer, 5, now))
        .await
        .unwrap();

    let early = sweep_archive(&bob.db, &owner, now + RETIRED_ARCHIVE_MS - 1)
        .await
        .unwrap();
    assert_eq!(early, ArchiveSweep::default());
    let swept = sweep_archive(&bob.db, &owner, now + RETIRED_ARCHIVE_MS)
        .await
        .unwrap();
    assert_eq!(
        (swept.prekeys, swept.rejections, swept.used_slots),
        (2, 1, 0)
    );
    assert_eq!(
        bob.count("signal_prekeys").await,
        2 + 2 * i64::from(SLOT_COUNT)
    );
    let swept = sweep_archive(&bob.db, &owner, now + OPK_STALE_MS)
        .await
        .unwrap();
    assert_eq!(swept.used_slots, 1);
}
