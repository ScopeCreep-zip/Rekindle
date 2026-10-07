use super::*;

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.split_whitespace().collect::<String>()).unwrap()
}

/// RFC 9605 Appendix C.1: every header vector encodes and decodes.
#[test]
fn rfc9605_c1_headers() {
    let vectors = include_str!("rfc9605_c1.txt");
    let mut count = 0;
    for line in vectors.lines().filter(|l| !l.starts_with('#')) {
        let mut parts = line.split(' ');
        let kid = u64::from_str_radix(parts.next().unwrap(), 16).unwrap();
        let ctr = u64::from_str_radix(parts.next().unwrap(), 16).unwrap();
        let header = unhex(parts.next().unwrap());

        let mut encoded = Vec::new();
        encode_header(Kid(kid), ctr, &mut encoded);
        assert_eq!(encoded, header, "encode kid={kid:#x} ctr={ctr:#x}");
        assert_eq!(
            parse_header(&header).unwrap(),
            (Kid(kid), ctr, header.len()),
            "decode kid={kid:#x} ctr={ctr:#x}"
        );
        count += 1;
    }
    assert_eq!(count, 289, "all Appendix C.1 vectors");
}

/// RFC 9605 Appendix C.3, cipher suite 0x0005.
#[test]
fn rfc9605_c3_aes_256_gcm_sha512_128() {
    let kid = Kid(0x123);
    let ctr = 0x4567;
    let base_key = unhex("000102030405060708090a0b0c0d0e0f");
    let metadata = unhex("4945544620534672616d65205747");
    let pt = unhex("64726166742d696574662d736672616d652d656e63");
    let ct = unhex(
        "990123456794f509d36e9beacb0e261d
         99c7d1e972f1fed787d4049f17ca2135
         3c1cc24d56ceabced279",
    );

    let key = derive(&base_key, kid);
    assert_eq!(
        key.key.as_ref(),
        unhex(
            "d3e27b0d4a5ae9e55df01a70e6d4d28d
             969b246e2936f4b7a5d9b494da6b9633"
        )
        .as_slice()
    );
    assert_eq!(key.salt.as_slice(), unhex("84991c167b8cd23c93708ec7"));
    assert_eq!(
        nonce(&key.salt, ctr).as_slice(),
        unhex("84991c167b8cd23c9370cba0")
    );

    assert_eq!(seal(&key, kid, ctr, &metadata, &pt).unwrap(), ct);
    assert_eq!(open(&key, &ct, &metadata).unwrap(), pt);
}

#[test]
fn open_rejects_tampering_and_wrong_metadata() {
    let key = derive(&[7u8; 32], Kid(9));
    let frame = seal(&key, Kid(9), 1, b"meta", b"audio").unwrap();

    assert_eq!(open(&key, &frame, b"other"), Err(SframeError::Open));
    let mut flipped = frame.clone();
    *flipped.last_mut().unwrap() ^= 1;
    assert_eq!(open(&key, &flipped, b"meta"), Err(SframeError::Open));
    // The header is authenticated: a different CTR breaks the tag.
    let mut reheadered = Vec::new();
    encode_header(Kid(9), 2, &mut reheadered);
    reheadered.extend_from_slice(&frame[2..]);
    assert_eq!(open(&key, &reheadered, b"meta"), Err(SframeError::Open));
    assert_eq!(open(&key, &[], b"meta"), Err(SframeError::Header));
    assert_eq!(open(&key, &[0x80], b"meta"), Err(SframeError::Header));
}

#[test]
fn media_kid_layout() {
    let kid = media_kid(0x00ab_cdef_0123_4567, 0x1_0203);
    assert_eq!(kid_tag(kid), 0x00ab_cdef_0123_4567);
    assert_eq!(kid_generation_low(kid), 0x03);
    assert!(kid_names_generation(kid, 0x0203));
    assert!(!kid_names_generation(kid, 0x0204));
    // The tag is 56 bits; higher bits never reach the KID.
    assert_eq!(kid_tag(media_kid(u64::MAX, 0)), (1 << 56) - 1);
}

/// Two senders sharing a scope secret, even with the same tag and CTR,
/// encrypt under different keys (§4.4.1: one base_key per sender).
#[test]
fn senders_sharing_a_scope_secret_get_distinct_keys() {
    let scope = [3u8; 32];
    let kid = media_kid(42, 0);
    let alice = media_key(&scope, &[1u8; 32], kid);
    let bob = media_key(&scope, &[2u8; 32], kid);
    let frame = seal(&alice, kid, 0, b"", b"hello").unwrap();
    assert!(open(&bob, &frame, b"").is_err());
    assert_eq!(
        open(&media_key(&scope, &[1u8; 32], kid), &frame, b"").unwrap(),
        b"hello"
    );
    // A new session tag gives a new key too.
    let next_session = media_key(&scope, &[1u8; 32], media_kid(43, 0));
    assert!(open(&next_session, &frame, b"").is_err());
}

#[test]
fn media_sender_never_repeats_a_counter() {
    let sender = std::sync::Arc::new(SframeSender::fresh());
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let sender = std::sync::Arc::clone(&sender);
            std::thread::spawn(move || (0..1000).map(|_| sender.next_ctr()).collect::<Vec<_>>())
        })
        .collect();
    let mut all: Vec<u64> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 4000);
    assert_eq!(kid_generation_low(sender.kid(5)), 5);
}
