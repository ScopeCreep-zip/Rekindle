//! The bundles over `signal_prekeys`: the long-lived keys are minted once,
//! each handout has fresh one-time keys under unused ids, and the
//! unclaimed one-time keys are capped.

use rekindle_crypto::signal::{PreKeyBundle, MAX_UNCLAIMED_ONE_TIME};

use super::*;
use crate::test_support::Party;

async fn ids(party: &Party, kind: PrekeyKind) -> Vec<u32> {
    let owner = identity_text(&party.local.owner());
    party
        .db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT id FROM signal_prekeys WHERE owner_key = ?1 AND kind = ?2 ORDER BY rowid",
            )?;
            let ids = stmt
                .query_map(params![owner, kind.as_str()], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<u32>>>();
            ids
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn the_current_bundle_is_minted_once_and_carries_no_one_time_keys() {
    let bob = Party::new(1);
    let first = current_bundle(&bob.db, &bob.local).await.unwrap();
    let again = current_bundle(&bob.db, &bob.local).await.unwrap();
    assert_eq!(first.to_wire(), again.to_wire());
    assert_eq!(first.identity_key, bob.local.owner());
    assert!(first.one_time_prekey.is_none() && first.pqpk_ot.is_none());
    assert_eq!(ids(&bob, PrekeyKind::Spk).await, [SPK_ID]);
    assert_eq!(ids(&bob, PrekeyKind::PqLr).await, [PQ_LR_ID]);

    let wire = PreKeyBundle::from_wire(&first.to_wire()).unwrap();
    assert_eq!(wire.to_wire(), first.to_wire(), "the one wire form");
}

#[tokio::test]
async fn each_handout_has_fresh_one_time_keys_beside_the_current_ones() {
    let bob = Party::new(1);
    let current = current_bundle(&bob.db, &bob.local).await.unwrap();
    let a = handout_bundle(&bob.db, &bob.local).await.unwrap();
    let b = handout_bundle(&bob.db, &bob.local).await.unwrap();
    for handout in [&a, &b] {
        assert_eq!(handout.signed_prekey, current.signed_prekey);
        assert_eq!(handout.pqpk_lr, current.pqpk_lr);
        assert!(handout.one_time_prekey_id.is_some_and(|id| id != 0));
        assert!(handout.pqpk_ot_id.is_some_and(|id| id != 0));
        assert!(handout.pqpk_ot_signature.is_some());
    }
    assert_ne!(a.one_time_prekey, b.one_time_prekey);
    assert_ne!(a.pqpk_ot, b.pqpk_ot);
    let opks = ids(&bob, PrekeyKind::Opk).await;
    assert_eq!(
        opks,
        [a.one_time_prekey_id.unwrap(), b.one_time_prekey_id.unwrap()]
    );
    assert_eq!(ids(&bob, PrekeyKind::PqOpk).await.len(), 2);
}

#[tokio::test]
async fn unclaimed_one_time_keys_beyond_the_cap_are_evicted_oldest_first() {
    let bob = Party::new(1);
    let mut handed = Vec::new();
    for _ in 0..MAX_UNCLAIMED_ONE_TIME + 2 {
        handed.push(
            handout_bundle(&bob.db, &bob.local)
                .await
                .unwrap()
                .one_time_prekey_id
                .unwrap(),
        );
    }
    let kept = ids(&bob, PrekeyKind::Opk).await;
    assert_eq!(kept.len(), MAX_UNCLAIMED_ONE_TIME);
    assert_eq!(kept, handed[2..]);
    assert_eq!(
        ids(&bob, PrekeyKind::PqOpk).await.len(),
        MAX_UNCLAIMED_ONE_TIME
    );
    assert_eq!(ids(&bob, PrekeyKind::Spk).await.len(), 1, "never evicted");
}

#[tokio::test]
async fn unclaimed_one_time_keys_are_counted() {
    let party = Party::new(5);
    assert_eq!(
        unclaimed_one_time(&party.db, &party.local.owner())
            .await
            .unwrap(),
        (0, 0)
    );
    handout_bundle(&party.db, &party.local).await.unwrap();
    handout_bundle(&party.db, &party.local).await.unwrap();
    assert_eq!(
        unclaimed_one_time(&party.db, &party.local.owner())
            .await
            .unwrap(),
        (2, 2)
    );
}
