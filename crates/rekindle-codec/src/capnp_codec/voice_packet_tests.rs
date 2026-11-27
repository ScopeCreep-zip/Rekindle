use super::*;
use crate::capnp_codec::SignedWire;
use rekindle_secrets::ed25519_dalek::SigningKey;

fn signed() -> (VoicePacket, SigningKey) {
    let key = SigningKey::from_bytes(&[5u8; 32]);
    let mut packet = VoicePacket {
        sender_key: key.verifying_key().to_bytes().to_vec(),
        sequence: 7,
        timestamp: 1_000,
        sframe: vec![0x12, 0xaa, 0xbb],
        sig: Vec::new(),
    };
    packet.sign(&key);
    (packet, key)
}

#[test]
fn round_trip_and_verify() {
    let (packet, _) = signed();
    let decoded = VoicePacket::decode(&packet.encode().unwrap()).unwrap();
    assert_eq!(decoded, packet);
    assert!(decoded.verify().is_ok());
}

#[test]
fn every_signed_field_is_covered() {
    let (packet, _) = signed();
    let mutations: [fn(&mut VoicePacket); 3] = [
        |p| p.sequence += 1,
        |p| p.timestamp += 1,
        |p| p.sframe[0] ^= 1,
    ];
    for mutate in mutations {
        let mut tampered = packet.clone();
        mutate(&mut tampered);
        assert!(tampered.verify().is_err());
    }
}
