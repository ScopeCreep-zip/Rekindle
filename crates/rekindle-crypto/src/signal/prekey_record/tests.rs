//! B1's maintenance plan, B2's pick and the minted record, each run with
//! the seeded generator so a seed fixes the outcome.

use std::collections::BTreeMap;

use rekindle_types::dht_layout::prekeys::{slot_subkey, FIRST_SLOT, SLOT_COUNT};

use super::*;
use crate::signal::test_support::TestRandom;
use crate::signal::SessionRecord;
use crate::Identity;

const NOW: i64 = 1_000 * DAY_MS;

fn head(issued_at_ms: i64) -> LocalHead {
    LocalHead {
        spk_id: 1,
        pq_lr_id: 2,
        issued_at_ms,
    }
}

/// A record whose head and every slot were minted at `at`.
fn full(at: i64) -> LocalPrekeys {
    LocalPrekeys {
        head: Some(head(at)),
        slots: (0..SLOT_COUNT)
            .map(|i| (slot_subkey(i), SlotState::Live { created_at_ms: at }))
            .collect(),
    }
}

fn fills(actions: &[PrekeyAction]) -> Vec<u32> {
    actions
        .iter()
        .filter_map(|a| match a {
            PrekeyAction::FillSlot { subkey } => Some(*subkey),
            PrekeyAction::RotateHead => None,
        })
        .collect()
}

#[test]
fn a_fresh_full_record_needs_nothing() {
    assert!(plan_maintenance(NOW, &full(NOW - 1)).is_empty());
}

#[test]
fn an_empty_record_is_minted_whole() {
    let actions = plan_maintenance(NOW, &LocalPrekeys::default());
    assert_eq!(actions[0], PrekeyAction::RotateHead);
    assert_eq!(
        fills(&actions),
        (0..SLOT_COUNT).map(slot_subkey).collect::<Vec<_>>()
    );
}

#[test]
fn the_head_rotates_at_two_days() {
    let just_before = plan_maintenance(NOW, &full(NOW - SIGNED_ROTATE_AFTER_MS + 1));
    assert!(just_before.is_empty());
    let mut local = full(NOW - 1);
    local.head = Some(head(NOW - SIGNED_ROTATE_AFTER_MS));
    assert_eq!(plan_maintenance(NOW, &local), [PrekeyAction::RotateHead]);
    // A head left long past 14 d (maintenance was failing) rotates by the
    // same rule before anything republishes it.
    local.head = Some(head(NOW - 3 * SIGNED_MAX_AGE_MS));
    assert_eq!(plan_maintenance(NOW, &local), [PrekeyAction::RotateHead]);
}

#[test]
fn a_consumed_slot_is_refilled_at_once() {
    let mut local = full(NOW - 1);
    local.slots.insert(FIRST_SLOT + 4, SlotState::Consumed);
    assert_eq!(
        plan_maintenance(NOW, &local),
        [PrekeyAction::FillSlot {
            subkey: FIRST_SLOT + 4
        }]
    );
}

#[test]
fn a_stale_slot_is_replaced_at_ninety_days() {
    let mut local = full(NOW - 1);
    local.slots.insert(
        FIRST_SLOT,
        SlotState::Live {
            created_at_ms: NOW - OPK_STALE_MS,
        },
    );
    local.slots.insert(
        FIRST_SLOT + 1,
        SlotState::Live {
            created_at_ms: NOW - OPK_STALE_MS + 1,
        },
    );
    assert_eq!(
        plan_maintenance(NOW, &local),
        [PrekeyAction::FillSlot { subkey: FIRST_SLOT }]
    );
}

/// Empty slots wait until fewer than ten are fresh, then the whole batch
/// is filled back to a hundred.
#[test]
fn empty_slots_are_topped_up_below_the_watermark() {
    let live = |n: u32| LocalPrekeys {
        head: Some(head(NOW - 1)),
        slots: (0..n)
            .map(|i| {
                (
                    slot_subkey(i),
                    SlotState::Live {
                        created_at_ms: NOW - 1,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
    };
    let at_watermark = u32::try_from(LOW_SLOT_WATERMARK).unwrap();
    assert!(plan_maintenance(NOW, &live(at_watermark)).is_empty());
    let below = plan_maintenance(NOW, &live(at_watermark - 1));
    assert_eq!(
        fills(&below),
        (at_watermark - 1..SLOT_COUNT)
            .map(slot_subkey)
            .collect::<Vec<_>>()
    );
}

#[test]
fn ids_are_non_zero_u31() {
    let rng = TestRandom::new(9);
    for _ in 0..10_000 {
        assert!(is_prekey_id(random_id(&rng)));
    }
    assert!(!is_prekey_id(0));
    assert!(!is_prekey_id(PREKEY_ID_MAX + 1));
    assert!(is_prekey_id(PREKEY_ID_MAX));
}

#[test]
fn the_rejection_delay_is_two_to_ten_seconds() {
    let rng = TestRandom::new(10);
    let delays: Vec<_> = (0..2_000).map(|_| rejection_delay_ms(&rng)).collect();
    assert!(delays.iter().all(|d| (2_000..=10_000).contains(d)));
    assert!(delays.iter().any(|d| *d < 3_000) && delays.iter().any(|d| *d > 9_000));
}

/// A minted set fills every slot subkey once, with ids distinct per kind,
/// and every value verifies under the identity.
#[test]
fn a_minted_set_is_a_whole_verified_record() {
    let bob = Identity::generate();
    let rng = TestRandom::new(11);
    let set = mint_set(&bob, &rng, NOW);
    assert_eq!(set.head.record.issued_at, NOW);
    let subkeys: Vec<_> = set.slots.iter().map(|s| s.subkey).collect();
    assert_eq!(
        subkeys,
        (0..SLOT_COUNT).map(slot_subkey).collect::<Vec<_>>()
    );
    let mut curve: Vec<_> = set.slots.iter().map(|s| s.opk_id).collect();
    curve.push(set.head.spk_id);
    let mut pq: Vec<_> = set.slots.iter().map(|s| s.pq_opk_id).collect();
    pq.push(set.head.pq_lr_id);
    for ids in [&mut curve, &mut pq] {
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "ids are distinct per kind");
        assert!(ids.iter().all(|id| is_prekey_id(*id)));
    }
    let view = RecordView::new(
        &bob.public_key_bytes(),
        set.head.record.clone(),
        set.slots.iter().map(|s| (s.subkey, s.record.clone())),
        NOW,
    )
    .unwrap();
    assert_eq!(view.slots().len(), set.slots.len());
}

/// Re-signing the stored public keys reproduces the published value, so
/// the reconciliation can compare bytes.
#[test]
fn re_signing_stored_keys_reproduces_the_value() {
    let bob = Identity::generate();
    let rng = TestRandom::new(12);
    let head = mint_head(&bob, &rng, 3, 4, NOW);
    let again = sign_head(&bob, 3, &head.spk.public, 4, &head.pq_lr.public, NOW).unwrap();
    assert_eq!(again, head.record);
    let slot = mint_slot(&bob, &rng, 7, 5, 6);
    let again = sign_slot(&bob, 5, &slot.opk.public, 6, &slot.pq_opk.public).unwrap();
    assert_eq!(again, slot.record);
    assert!(sign_slot(&bob, 5, &[0; 31], 6, &slot.pq_opk.public).is_err());
}

fn view_of(bob: &Identity, set: &MintedSet) -> RecordView {
    RecordView::new(
        &bob.public_key_bytes(),
        set.head.record.clone(),
        set.slots.iter().map(|s| (s.subkey, s.record.clone())),
        NOW,
    )
    .unwrap()
}

#[test]
fn the_pick_is_uniform_over_unused_slots_and_deterministic_per_seed() {
    let bob = Identity::generate();
    let set = mint_set(&bob, &TestRandom::new(13), NOW);
    let view = view_of(&bob, &set);
    let used: UsedIds = set.slots[..90].iter().map(|s| s.opk_id).collect();

    let rng = TestRandom::new(14);
    let mut seen = BTreeMap::new();
    for _ in 0..2_000 {
        match pick_slot(&view, &used, &rng) {
            Pick::Slot { subkey, opk_id } => {
                assert!(!used.contains(&opk_id), "a used slot was picked");
                *seen.entry(subkey).or_insert(0) += 1;
            }
            Pick::LastResortOnly => panic!("ten slots are unused"),
        }
    }
    assert_eq!(seen.len(), 10, "every unused slot is reachable");
    assert!(seen.values().all(|n| (120..=280).contains(n)), "{seen:?}");

    let a = pick_slot(&view, &used, &TestRandom::new(15));
    let b = pick_slot(&view, &used, &TestRandom::new(15));
    assert_eq!(a, b, "a seed fixes the pick");
}

/// With every slot used (or none listed) the pick is the head alone, and
/// its bundle opens a session on the last-resort key.
#[test]
fn with_no_unused_slot_the_pick_is_the_head_alone() {
    let bob = Identity::generate();
    let alice = Identity::generate();
    let rng = TestRandom::new(16);
    let set = mint_set(&bob, &rng, NOW);
    let view = view_of(&bob, &set);
    let used: UsedIds = view.listed_opk_ids().collect();
    let pick = pick_slot(&view, &used, &rng);
    assert_eq!(pick, Pick::LastResortOnly);
    let bundle = view.bundle(&pick).unwrap();
    let (_, init) = SessionRecord::new(bob.public_key_bytes())
        .initiate(&alice, &bundle, b"hi", b"", &rng)
        .unwrap();
    assert_eq!(init.one_time_prekey_id, None);
    assert_eq!(
        init.pq_prekey,
        crate::signal::PqPrekeyId::LastResort(set.head.pq_lr_id)
    );
    assert_eq!(init.signed_prekey_id, set.head.spk_id);

    let empty = RecordView::new(&bob.public_key_bytes(), set.head.record.clone(), [], NOW).unwrap();
    assert_eq!(
        pick_slot(&empty, &UsedIds::new(), &rng),
        Pick::LastResortOnly
    );
}

/// The view is checked under the pinned identity: a head signed by anyone
/// else is refused, and a slot that does not verify (or sits outside the
/// slot range) is left out, never picked.
#[test]
fn the_view_keeps_only_what_the_pinned_identity_signed() {
    let (bob, mallory) = (Identity::generate(), Identity::generate());
    let rng = TestRandom::new(17);
    let set = mint_set(&bob, &rng, NOW);
    assert!(RecordView::new(
        &mallory.public_key_bytes(),
        set.head.record.clone(),
        [],
        NOW
    )
    .is_err());

    let forged = mint_slot(&mallory, &rng, FIRST_SLOT, 77, 78);
    let view = RecordView::new(
        &bob.public_key_bytes(),
        set.head.record.clone(),
        [
            (FIRST_SLOT, forged.record),
            (0, set.slots[1].record.clone()),
            (FIRST_SLOT + 2, set.slots[2].record.clone()),
        ],
        NOW,
    )
    .unwrap();
    assert_eq!(
        view.listed_opk_ids().collect::<Vec<_>>(),
        [set.slots[2].opk_id]
    );
    let pick = pick_slot(&view, &UsedIds::new(), &rng);
    assert_eq!(
        pick,
        Pick::Slot {
            subkey: FIRST_SLOT + 2,
            opk_id: set.slots[2].opk_id
        }
    );
    let bundle = view.bundle(&pick).unwrap();
    assert_eq!(bundle.one_time_prekey_id, Some(set.slots[2].opk_id));
    assert!(view
        .bundle(&Pick::Slot {
            subkey: FIRST_SLOT,
            opk_id: 77
        })
        .is_err());
}
