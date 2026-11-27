use super::*;
use rekindle_types::dht_layout::prekeys::slot_subkey;

fn healthy() -> PrekeyStatus {
    PrekeyStatus {
        device_id: None,
        record_key: None,
        spk_id: Some(1),
        spk_age_secs: Some(3_600),
        pq_lr_id: Some(2),
        lr_age_secs: Some(3_600),
        fresh_slots: SLOT_COUNT,
        consumed_slots: 0,
        component_lens: None,
    }
}

#[test]
fn the_problem_names_what_maintenance_failed_to_keep() {
    assert_eq!(prekey_problem(&healthy()), None);
    let low = PrekeyStatus {
        fresh_slots: 3,
        ..healthy()
    };
    assert_eq!(
        prekey_problem(&low),
        Some(format!("3/{SLOT_COUNT} fresh slots"))
    );
    let stale = PrekeyStatus {
        spk_age_secs: Some(15 * 86_400),
        ..healthy()
    };
    assert!(prekey_problem(&stale).unwrap().contains("15 days"));
    let none = PrekeyStatus {
        spk_age_secs: None,
        ..healthy()
    };
    assert!(prekey_problem(&none).is_some());
}

fn value(subkey: u32) -> Publish {
    Publish {
        subkey,
        value: vec![u8::try_from(subkey % 256).unwrap()],
    }
}

#[test]
fn the_first_contact_part_is_the_head_and_the_watermark_slots() {
    let slots = u32::try_from(LOW_SLOT_WATERMARK).unwrap() + 5;
    let mut values: Vec<Publish> = (0..slots).map(|i| value(slot_subkey(i))).collect();
    values.insert(3, value(HEAD));
    let (now, later) = split_first_contact(values);
    assert_eq!(now.len(), 1 + LOW_SLOT_WATERMARK);
    assert!(now.iter().any(|p| p.subkey == HEAD));
    assert_eq!(later.len(), 5);
    assert!(later.iter().all(|p| p.subkey != HEAD));

    let (now, later) = split_first_contact(vec![value(slot_subkey(0))]);
    assert_eq!((now.len(), later.len()), (1, 0));
}
