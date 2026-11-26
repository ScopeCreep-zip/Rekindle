use super::*;

#[test]
fn one_publish_names_the_head_and_its_slots() {
    let mut written = Written::default();
    for subkey in [3, HEAD, 1, 2] {
        written.add(subkey);
    }
    let Some(CryptoEvent::PrekeyRecordPublished { head, slots }) = written.into_event() else {
        panic!("one event");
    };
    assert!(head);
    assert_eq!(slots, vec![1, 2, 3]);
    assert!(
        Written::default().into_event().is_none(),
        "nothing written, no event"
    );
}

#[test]
fn the_device_list_names_one_device() {
    let list = device_list("VLD0:abc", &"0f".repeat(16)).unwrap();
    let decoded = rekindle_codec::capnp_codec::identity::decode_prekey_record_list(&list).unwrap();
    assert_eq!(decoded.devices.len(), 1);
    assert_eq!(decoded.devices[0].record_key, "VLD0:abc");
    assert!(device_list("VLD0:abc", "short").is_err());
}
