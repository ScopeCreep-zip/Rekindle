use super::blake3_hex;

#[test]
fn known_answer_matches_the_blake3_spec() {
    // BLAKE3 of the empty input — official test vector, so this
    // pins the encoding (lowercase hex, untruncated) and not just
    // "whatever the library returned today".
    assert_eq!(
        blake3_hex(b""),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
}

#[test]
fn output_is_64_lowercase_hex_chars() {
    let hash = blake3_hex(b"rekindle");
    assert_eq!(hash.len(), 64);
    assert!(hash
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn distinct_inputs_hash_differently() {
    assert_ne!(blake3_hex(b"a"), blake3_hex(b"b"));
}
