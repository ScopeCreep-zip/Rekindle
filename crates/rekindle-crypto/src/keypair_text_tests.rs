use super::*;

#[test]
fn text_and_wire_round_trip() {
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let text = keypair_text_from_signing(&signing);
    assert!(text.starts_with("VLD0:"));
    let (public, secret) = parse_keypair_text(&text).unwrap();
    assert_eq!(public, signing.verifying_key().to_bytes());
    assert_eq!(secret, signing.to_bytes());
    let wire = keypair_wire(&text).unwrap();
    assert_eq!(keypair_text_from_wire(&wire).unwrap(), text);
}

#[test]
fn malformed_text_is_refused() {
    assert!(parse_keypair_text("VLD1:AAAA:AAAA").is_err());
    assert!(parse_keypair_text("VLD0:AAAA").is_err());
    assert!(parse_keypair_text("VLD0:AAAA:AAAA:AAAA").is_err());
    assert!(parse_keypair_text("VLD0:!!:??").is_err());
    assert!(keypair_text_from_wire(&[0u8; 63]).is_err());
}

#[test]
fn a_slot_keypair_is_the_seed_derivation() {
    let seed = [3u8; 32];
    let signing = rekindle_secrets::derive::derive_slot_keypair(&seed, 5).unwrap();
    assert_eq!(
        slot_keypair_text(&seed, 5).unwrap(),
        keypair_text_from_signing(&signing)
    );
}
