use super::*;
use crate::repo::fixture::{self, OWNER};

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn skin(hash: &str) -> NewSkin<'_> {
    NewSkin {
        hash,
        manifest_json: "{}",
        bytes: b"pk",
        imported_at: 7,
        origin: Origin::Import,
    }
}

#[test]
fn a_package_round_trips_once_and_is_removed() {
    let conn = fixture::conn();
    assert!(insert(&conn, OWNER, &skin(HASH)).unwrap());
    assert!(
        !insert(&conn, OWNER, &skin(HASH)).unwrap(),
        "kept as it was"
    );
    assert!(contains(&conn, OWNER, HASH).unwrap());
    assert_eq!(
        bytes(&conn, OWNER, HASH).unwrap().as_deref(),
        Some(&b"pk"[..])
    );
    let rows = list(&conn, OWNER).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].imported_at, rows[0].origin.as_str()),
        (7, "import")
    );
    assert!(remove(&conn, OWNER, HASH).unwrap());
    assert!(!remove(&conn, OWNER, HASH).unwrap());
    assert!(!contains(&conn, OWNER, HASH).unwrap());
    assert_eq!(bytes(&conn, OWNER, HASH).unwrap(), None);
}

#[test]
fn the_checks_refuse_a_bad_hash_origin_or_size() {
    let conn = fixture::conn();
    let upper = HASH.to_uppercase();
    let short = &HASH[..63];
    let not_hex = format!("{}g", &HASH[..63]);
    for bad in [upper.as_str(), short, not_hex.as_str(), ""] {
        assert!(insert(&conn, OWNER, &skin(bad)).is_err(), "{bad:?}");
    }
    let refused = conn.execute(
        "INSERT INTO skin_packages (owner_key, hash, manifest_json, bytes, imported_at, origin) \
             VALUES (?1, ?2, '{}', x'00', 0, 'stolen')",
        params![OWNER, HASH],
    );
    assert!(refused.is_err(), "unknown origin");
    let big = vec![0_u8; 16 * 1024 * 1024 + 1];
    let oversize = NewSkin {
        bytes: &big,
        ..skin(HASH)
    };
    assert!(insert(&conn, OWNER, &oversize).is_err(), "over 16 MiB");
    assert!(list(&conn, OWNER).unwrap().is_empty());
}
