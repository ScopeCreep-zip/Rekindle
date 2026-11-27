use super::*;

#[test]
fn exact_32_parses() {
    assert_eq!(to_32(&[7u8; 32], "k").unwrap(), [7u8; 32]);
}

#[test]
fn short_slice_errors_with_label_and_len() {
    let err = to_32(&[0u8; 31], "signed prekey").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("signed prekey"), "{msg}");
    assert!(msg.contains("31"), "{msg}");
}

#[test]
fn long_slice_errors_instead_of_truncating() {
    // The old daemon-track helper took the first 32 bytes of longer
    // input; key material silently losing bytes is never right.
    let err = to_32(&[0u8; 40], "identity key").unwrap_err();
    assert!(err.to_string().contains("40"));
}
