//! The log scrubber (`rekindle_utils::log_scrub`) must remove every
//! veilid-core identifier as veilid-core itself formats it. Patterns
//! written against hand-typed examples drift when the upstream encoding
//! changes; these assertions use real values, so a veilid-core upgrade
//! that changes `Display`/`Debug` fails here instead of leaking keys into
//! log files.

use rekindle_utils::log_scrub::scrub;
use veilid_core::{
    BareNodeId, BareOpaqueRecordKey, BarePublicKey, BareRecordKey, BareRouteId, BareSharedSecret,
    BareSignature, NodeId, PublicKey, RecordKey, RouteId, Signature, CRYPTO_KIND_VLD0,
};

fn bytes<const N: usize>(seed: u8) -> [u8; N] {
    std::array::from_fn(|i| {
        let i = u8::try_from(i).expect("test arrays are at most 64 bytes");
        seed.wrapping_mul(31).wrapping_add(i).wrapping_mul(97)
    })
}

fn assert_scrubbed(rendered: &str, raw_parts: &[String]) {
    let line = format!("event value={rendered} other=1");
    let out = scrub(&line);
    for part in raw_parts {
        assert!(
            !out.contains(part.as_str()),
            "`{part}` survived scrubbing of `{line}` → `{out}`"
        );
    }
    assert!(
        out.contains("other=1"),
        "scrubbing ate surrounding text: `{out}`"
    );
}

#[test]
fn typed_and_bare_keys_in_display_and_debug() {
    let pk_bare = BarePublicKey::new(&bytes::<32>(1));
    let pk = PublicKey::new(CRYPTO_KIND_VLD0, pk_bare.clone());
    let node = NodeId::new(CRYPTO_KIND_VLD0, BareNodeId::new(&bytes::<32>(2)));
    let route = RouteId::new(CRYPTO_KIND_VLD0, BareRouteId::new(&bytes::<32>(3)));
    let sig = Signature::new(CRYPTO_KIND_VLD0, BareSignature::new(&bytes::<64>(4)));

    for (display, debug, bare) in [
        (pk.to_string(), format!("{pk:?}"), pk_bare.to_string()),
        (
            node.to_string(),
            format!("{node:?}"),
            node.value().to_string(),
        ),
        (
            route.to_string(),
            format!("{route:?}"),
            route.value().to_string(),
        ),
        (sig.to_string(), format!("{sig:?}"), sig.value().to_string()),
    ] {
        assert_scrubbed(&display, std::slice::from_ref(&bare));
        assert_scrubbed(&debug, std::slice::from_ref(&bare));
        assert_scrubbed(&bare, std::slice::from_ref(&bare));
    }
}

#[test]
fn record_keys_with_and_without_encryption_key() {
    let opaque = BareOpaqueRecordKey::new(&bytes::<32>(5));
    let secret = BareSharedSecret::new(&bytes::<32>(6));
    let plain = RecordKey::new(CRYPTO_KIND_VLD0, BareRecordKey::new(opaque.clone(), None));
    let encrypted = RecordKey::new(
        CRYPTO_KIND_VLD0,
        BareRecordKey::new(opaque.clone(), Some(secret.clone())),
    );
    let parts = [opaque.to_string(), secret.to_string()];
    for rendered in [
        plain.to_string(),
        format!("{plain:?}"),
        encrypted.to_string(),
        format!("{encrypted:?}"),
    ] {
        assert_scrubbed(&rendered, &parts);
    }
}
