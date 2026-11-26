#[test]
fn a_drop_round_trips_its_envelopes_and_drops_a_truncated_tail() {
    let value = super::encode(&[b"one".to_vec(), b"two!".to_vec()]);
    assert_eq!(super::decode(&value), vec![&b"one"[..], &b"two!"[..]]);
    assert_eq!(super::decode(&value[..value.len() - 1]), vec![&b"one"[..]]);
}

#[test]
fn a_drop_holds_only_what_fits_one_subkey() {
    let big = vec![0u8; super::MAX_SUBKEY_VALUE_LEN / 2];
    let value = super::encode(&[big.clone(), big.clone(), big]);
    assert_eq!(super::decode(&value).len(), 1);
    assert!(value.len() <= super::MAX_SUBKEY_VALUE_LEN);
}
