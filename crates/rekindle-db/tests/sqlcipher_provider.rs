//! SQLCipher's crypto provider per target and the at-rest format it keeps
//! (plan SC1 slice 3).
//!
//! Apple builds take CommonCrypto (`rusqlite/bundled-sqlcipher` without
//! vendored OpenSSL: libsqlite3-sys 0.37 `build.rs` defines
//! `SQLCIPHER_CRYPTO_CC` and links Security.framework); every other target
//! keeps the static OpenSSL from `openssl-src`. A system OpenSSL found
//! through `OPENSSL_DIR` (or `OPENSSL_LIB_DIR` + `OPENSSL_INCLUDE_DIR`)
//! would silently replace CommonCrypto on Apple and link `libcrypto`
//! dynamically; the provider test fails then.
//!
//! The page format does not depend on the provider (AES-256-CBC pages,
//! HMAC-SHA512 per page, PBKDF2-HMAC-SHA512 KDF, 4096-byte pages): the
//! fixture was written by the OpenSSL build before the switch and must open
//! and verify under whatever this build links.

use rusqlite::Connection;

/// The raw key the fixture was written with (`PRAGMA key = "x'5a…'"`, the
/// raw-key form `rekindle_db::open` and the vault use).
const FIXTURE_KEY: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

fn keyed(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "key", format!("x'{FIXTURE_KEY}'"))
        .unwrap();
    conn
}

fn text(conn: &Connection, pragma: &str) -> String {
    conn.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0))
        .unwrap()
}

/// The fixture's body: 10 000 bytes spanning several pages.
fn fixture_body() -> Vec<u8> {
    (0u32..10_000)
        .map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).unwrap())
        .collect()
}

fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = keyed(&dir.path().join("fresh.db"));
    conn.execute_batch("CREATE TABLE t(x)").unwrap();
    (dir, conn)
}

#[cfg(target_vendor = "apple")]
#[test]
fn apple_builds_use_commoncrypto() {
    let (_dir, conn) = fresh();
    assert_eq!(text(&conn, "cipher_provider"), "commoncrypto");
}

#[cfg(not(target_vendor = "apple"))]
#[test]
fn other_builds_use_the_vendored_openssl() {
    let (_dir, conn) = fresh();
    assert_eq!(text(&conn, "cipher_provider"), "openssl");
}

#[test]
fn the_cipher_settings_are_sqlcipher_4_defaults() {
    let (_dir, conn) = fresh();
    assert!(text(&conn, "cipher_version").starts_with("4."));
    assert_eq!(text(&conn, "kdf_iter"), "256000");
    assert_eq!(text(&conn, "cipher_page_size"), "4096");
    assert_eq!(text(&conn, "cipher_use_hmac"), "1");
    assert_eq!(text(&conn, "cipher_hmac_algorithm"), "HMAC_SHA512");
    assert_eq!(text(&conn, "cipher_kdf_algorithm"), "PBKDF2_HMAC_SHA512");
    assert_eq!(text(&conn, "cipher_plaintext_header_size"), "0");
}

#[test]
fn a_database_written_by_the_openssl_build_opens_and_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("written.db");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sqlcipher-openssl-written.db"
        ),
        &path,
    )
    .unwrap();
    let conn = keyed(&path);
    let (provider, cipher_version, body): (String, String, Vec<u8>) = conn
        .query_row(
            "SELECT provider, cipher_version, body FROM written_by",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(provider, "openssl");
    assert!(cipher_version.starts_with("4."));
    assert_eq!(body, fixture_body());
    // Every page's HMAC checks out under this build's provider.
    let mut stmt = conn.prepare("PRAGMA cipher_integrity_check").unwrap();
    let errors: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(errors.is_empty(), "{errors:?}");
}
