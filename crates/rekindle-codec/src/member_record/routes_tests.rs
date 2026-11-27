use super::*;

const AT: RecordScope<'static> = RecordScope {
    community_id: "VLD0:community",
    member_record: "VLD0:member",
};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

#[test]
fn a_worst_case_blob_round_trips_under_the_subkey_cap() {
    let routes = MemberRoutes {
        // Incompressible bytes, so packing cannot shrink them.
        general: (0..MEMBER_ROUTE_BLOB_MAX_BYTES)
            .map(|i| u8::try_from(i * 131 % 251 + 1).unwrap_or(1))
            .collect(),
        issued_at: u64::MAX,
    };
    let bytes = routes.encode(AT, &key(1)).expect("encodes");
    assert!(bytes.len() <= MEMBER_ROUTES_MAX_BYTES, "{}", bytes.len());
    const _: () = assert!(
        MEMBER_ROUTES_MAX_BYTES <= rekindle_types::dht_layout::member_record::MAX_SUBKEY_VALUE_LEN
    );
    let pseudonym = key(1).verifying_key().to_bytes();
    assert_eq!(
        decode_member_routes(&bytes, AT, &pseudonym).unwrap(),
        routes
    );
}

#[test]
fn another_signer_record_or_an_empty_blob_is_refused() {
    let routes = MemberRoutes {
        general: vec![7; 300],
        issued_at: 5,
    };
    let bytes = routes.encode(AT, &key(1)).unwrap();
    let other = key(2).verifying_key().to_bytes();
    assert!(decode_member_routes(&bytes, AT, &other).is_err());
    let moved = RecordScope {
        member_record: "VLD0:other",
        ..AT
    };
    let pseudonym = key(1).verifying_key().to_bytes();
    assert!(decode_member_routes(&bytes, moved, &pseudonym).is_err());
    let empty = MemberRoutes {
        general: Vec::new(),
        issued_at: 5,
    };
    assert!(empty.encode(AT, &key(1)).is_err());
    let over = MemberRoutes {
        general: vec![1; MEMBER_ROUTE_BLOB_MAX_BYTES + 1],
        issued_at: 5,
    };
    assert!(over.encode(AT, &key(1)).is_err());
}
