//! The identity's skin library, `skin_packages` (plan E7.7.2): each
//! package the skin helper validated, as its canonical bytes, keyed by
//! their BLAKE3 (64 lowercase hex). The Host validates; this module only
//! stores, and the table's CHECK constraints refuse a malformed hash, an oversize
//! package or an unknown origin.

use rusqlite::{params, Connection, OptionalExtension as _};

/// How a package entered the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `SkinImport`.
    Import,
    /// Received from a friend (E7.7.6).
    Received,
    /// Made in the editor (E7.7.5).
    Authored,
}

impl Origin {
    /// The stored text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Received => "received",
            Self::Authored => "authored",
        }
    }
}

/// One library row without its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinRow {
    pub hash: String,
    pub manifest_json: String,
    pub imported_at: i64,
    pub origin: String,
}

/// A package to store.
#[derive(Debug, Clone, Copy)]
pub struct NewSkin<'a> {
    pub hash: &'a str,
    pub manifest_json: &'a str,
    pub bytes: &'a [u8],
    pub imported_at: i64,
    pub origin: Origin,
}

/// Store `skin`; a package already in the library is kept as it was.
/// `true` when the row is new.
///
/// # Errors
/// The write failed, or a CHECK refused the row.
pub fn insert(conn: &Connection, owner_key: &str, skin: &NewSkin<'_>) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "INSERT INTO skin_packages (owner_key, hash, manifest_json, bytes, imported_at, origin) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(owner_key, hash) DO NOTHING",
        params![
            owner_key,
            skin.hash,
            skin.manifest_json,
            skin.bytes,
            skin.imported_at,
            skin.origin.as_str()
        ],
    )?;
    Ok(n == 1)
}

/// The library, oldest first.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<SkinRow>> {
    let mut stmt = conn.prepare(
        "SELECT hash, manifest_json, imported_at, origin FROM skin_packages \
         WHERE owner_key = ?1 ORDER BY imported_at, hash",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok(SkinRow {
            hash: row.get(0)?,
            manifest_json: row.get(1)?,
            imported_at: row.get(2)?,
            origin: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Whether the library holds `hash`.
///
/// # Errors
/// The query failed.
pub fn contains(conn: &Connection, owner_key: &str, hash: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM skin_packages WHERE owner_key = ?1 AND hash = ?2)",
        params![owner_key, hash],
        |row| row.get(0),
    )
}

/// The canonical bytes of `hash`, if the library holds it.
///
/// # Errors
/// The query failed.
pub fn bytes(conn: &Connection, owner_key: &str, hash: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    conn.query_row(
        "SELECT bytes FROM skin_packages WHERE owner_key = ?1 AND hash = ?2",
        params![owner_key, hash],
        |row| row.get(0),
    )
    .optional()
}

/// Remove `hash`; `true` when a row went.
///
/// # Errors
/// The write failed.
pub fn remove(conn: &Connection, owner_key: &str, hash: &str) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "DELETE FROM skin_packages WHERE owner_key = ?1 AND hash = ?2",
        params![owner_key, hash],
    )?;
    Ok(n == 1)
}

#[cfg(test)]
#[path = "skins_tests.rs"]
mod tests;
