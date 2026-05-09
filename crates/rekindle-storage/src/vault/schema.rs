//! Vault database schema — all CREATE TABLE statements and migrations.
//!
//! Schema version is tracked in the `schema_version` table. Migrations
//! are append-only (ALTER TABLE ADD COLUMN, CREATE TABLE IF NOT EXISTS).
//! No destructive migrations.

use rusqlite::Connection;

use crate::error::{StorageError, StorageResult};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

const SCHEMA_V1: &str = "
-- Schema version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL,
    migrated_at INTEGER NOT NULL
);

-- Cryptographic key material (signing key, DhtLog keypairs, prekey private, governance keypairs)
CREATE TABLE IF NOT EXISTS keys (
    label TEXT PRIMARY KEY NOT NULL,
    value BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

-- Triple Ratchet session state (opaque CBOR blob, entry-encrypted)
CREATE TABLE IF NOT EXISTS ratchet_sessions (
    session_id BLOB PRIMARY KEY NOT NULL,
    peer_key TEXT NOT NULL,
    direction INTEGER NOT NULL,
    session_data BLOB NOT NULL,
    spqr_active INTEGER NOT NULL DEFAULT 0,
    trust_level INTEGER NOT NULL DEFAULT 0,
    last_active INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ratchet_peer ON ratchet_sessions(peer_key);
CREATE INDEX IF NOT EXISTS idx_ratchet_active ON ratchet_sessions(last_active);

-- Skipped message keys (HE-DR: keyed by encrypted header_key + counter)
CREATE TABLE IF NOT EXISTS skipped_keys (
    session_id BLOB NOT NULL,
    header_key BLOB NOT NULL,
    counter INTEGER NOT NULL,
    message_key BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (session_id, header_key, counter)
);
CREATE INDEX IF NOT EXISTS idx_skipped_ttl ON skipped_keys(created_at);
CREATE INDEX IF NOT EXISTS idx_skipped_session ON skipped_keys(session_id);

-- Sent DM plaintext (forward secrecy: cannot re-decrypt our own outbound)
CREATE TABLE IF NOT EXISTS dm_sent (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_key TEXT NOT NULL,
    body BLOB NOT NULL,
    timestamp INTEGER NOT NULL,
    message_id TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS idx_dm_sent_peer ON dm_sent(peer_key, timestamp);

-- Received DM plaintext cache
CREATE TABLE IF NOT EXISTS dm_received (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_key TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    body BLOB NOT NULL,
    timestamp INTEGER NOT NULL,
    sequence INTEGER NOT NULL,
    message_id TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS idx_dm_recv_peer ON dm_received(peer_key, timestamp);

-- Channel message cache
CREATE TABLE IF NOT EXISTS channel_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    author_pseudonym TEXT NOT NULL,
    author_display_name TEXT NOT NULL,
    body BLOB NOT NULL,
    timestamp INTEGER NOT NULL,
    sequence INTEGER NOT NULL,
    message_id TEXT NOT NULL,
    mek_generation INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chan_msg ON channel_messages(community_id, channel_id, timestamp);
CREATE UNIQUE INDEX IF NOT EXISTS idx_chan_dedup ON channel_messages(community_id, channel_id, sequence);

-- MEK cache (per-channel encryption keys)
CREATE TABLE IF NOT EXISTS mek_cache (
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    key_bytes BLOB NOT NULL,
    cached_at INTEGER NOT NULL,
    PRIMARY KEY (community_id, channel_id, generation)
);

-- Friend display names
CREATE TABLE IF NOT EXISTS friend_names (
    peer_key TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Pending outbound DhtLog keys (bridge: friend request send → acceptance discovery)
CREATE TABLE IF NOT EXISTS pending_outbound_logs (
    target_profile_key TEXT PRIMARY KEY NOT NULL,
    outbound_log_key TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
";

pub fn create_all(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(SCHEMA_V1).map_err(|e| StorageError::VaultCreationFailed {
        reason: format!("schema: {e}"),
    })?;

    conn.execute(
        "INSERT INTO schema_version (id, version, migrated_at) VALUES (1, ?1, ?2)",
        rusqlite::params![CURRENT_SCHEMA_VERSION, timestamp_secs()],
    )
    .map_err(|e| StorageError::VaultCreationFailed {
        reason: format!("schema version: {e}"),
    })?;

    Ok(())
}

pub fn migrate(conn: &Connection) -> StorageResult<()> {
    let current: u32 = conn
        .query_row(
            "SELECT version FROM schema_version WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .map_err(|e| StorageError::VaultCorrupt {
            reason: format!("schema_version: {e}"),
        })?;

    if current == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    if current > CURRENT_SCHEMA_VERSION {
        return Err(StorageError::VaultCorrupt {
            reason: format!(
                "schema v{current} > code v{CURRENT_SCHEMA_VERSION} — downgrade not supported"
            ),
        });
    }

    // Migration chain: when v2 is needed, add: if current < 2 { migrate_v1_to_v2(conn)?; }

    conn.execute(
        "UPDATE schema_version SET version = ?1, migrated_at = ?2 WHERE id = 1",
        rusqlite::params![CURRENT_SCHEMA_VERSION, timestamp_secs()],
    )?;

    Ok(())
}

// Timestamps won't exceed i64::MAX until year ~292 billion.
#[allow(clippy::cast_possible_wrap)]
pub(crate) fn timestamp_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
