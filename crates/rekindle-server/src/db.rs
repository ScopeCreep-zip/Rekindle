use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// Server-side schema version. Bump when the schema changes.
const SERVER_SCHEMA_VERSION: i64 = 4;

/// Open (or create) the server `SQLite` database and run migrations.
pub fn open_server_db(path: &str) -> Result<Arc<Mutex<Connection>>, String> {
    let conn = Connection::open(path).map_err(|e| format!("failed to open server db: {e}"))?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| format!("failed to set WAL mode: {e}"))?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("failed to enable foreign keys: {e}"))?;

    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);

    if current != SERVER_SCHEMA_VERSION {
        if current != 0 {
            tracing::info!(
                old = current,
                new = SERVER_SCHEMA_VERSION,
                "server schema version mismatch — recreating"
            );
            drop_all_tables(&conn)?;
        }
        conn.execute_batch(SERVER_SCHEMA)
            .map_err(|e| format!("failed to run server schema: {e}"))?;
        conn.pragma_update(None, "user_version", SERVER_SCHEMA_VERSION)
            .map_err(|e| format!("failed to set schema version: {e}"))?;
    }

    Ok(Arc::new(Mutex::new(conn)))
}

/// Drop every user table so the schema can be cleanly re-applied.
fn drop_all_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| format!("failed to disable fks: {e}"))?;

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| format!("failed to list tables: {e}"))?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("failed to query tables: {e}"))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);

    for table in &tables {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\";"))
            .map_err(|e| format!("failed to drop table {table}: {e}"))?;
    }

    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("failed to re-enable fks: {e}"))?;

    Ok(())
}

const SERVER_SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS hosted_communities (
    id TEXT PRIMARY KEY,
    dht_record_key TEXT NOT NULL,
    owner_keypair_hex TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    creator_pseudonym TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS server_members (
    community_id TEXT NOT NULL REFERENCES hosted_communities(id) ON DELETE CASCADE,
    pseudonym_key_hex TEXT NOT NULL,
    display_name TEXT,
    joined_at INTEGER NOT NULL,
    signal_session_data BLOB,
    route_blob BLOB,
    PRIMARY KEY (community_id, pseudonym_key_hex)
);

CREATE TABLE IF NOT EXISTS server_mek (
    community_id TEXT NOT NULL REFERENCES hosted_communities(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL,
    key_bytes BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (community_id, generation)
);

CREATE TABLE IF NOT EXISTS server_channels (
    community_id TEXT NOT NULL REFERENCES hosted_communities(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL CHECK(channel_type IN ('text','voice')),
    sort_order INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (community_id, id)
);

CREATE TABLE IF NOT EXISTS server_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    sender_pseudonym TEXT NOT NULL,
    ciphertext BLOB NOT NULL,
    mek_generation INTEGER NOT NULL,
    timestamp INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_server_messages
    ON server_messages(community_id, channel_id, timestamp);

CREATE TABLE IF NOT EXISTS banned_members (
    community_id TEXT NOT NULL REFERENCES hosted_communities(id) ON DELETE CASCADE,
    pseudonym_key_hex TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    banned_at INTEGER NOT NULL,
    PRIMARY KEY (community_id, pseudonym_key_hex)
);

-- Role definitions per community
CREATE TABLE IF NOT EXISTS server_roles (
    community_id TEXT NOT NULL REFERENCES hosted_communities(id) ON DELETE CASCADE,
    id INTEGER NOT NULL,
    name TEXT NOT NULL,
    color INTEGER NOT NULL DEFAULT 0,
    permissions INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL DEFAULT 0,
    hoist INTEGER NOT NULL DEFAULT 0,
    mentionable INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (community_id, id)
);

-- Junction table: which roles a member has
CREATE TABLE IF NOT EXISTS server_member_roles (
    community_id TEXT NOT NULL,
    pseudonym_key_hex TEXT NOT NULL,
    role_id INTEGER NOT NULL,
    PRIMARY KEY (community_id, pseudonym_key_hex, role_id),
    FOREIGN KEY (community_id, pseudonym_key_hex)
        REFERENCES server_members(community_id, pseudonym_key_hex) ON DELETE CASCADE
);

-- Per-channel permission overwrites
CREATE TABLE IF NOT EXISTS server_channel_overwrites (
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK(target_type IN ('role','member')),
    target_id TEXT NOT NULL,
    allow_bits INTEGER NOT NULL DEFAULT 0,
    deny_bits INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (community_id, channel_id, target_type, target_id),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES server_channels(community_id, id) ON DELETE CASCADE
);

-- Member timeouts
CREATE TABLE IF NOT EXISTS server_member_timeouts (
    community_id TEXT NOT NULL,
    pseudonym_key_hex TEXT NOT NULL,
    timeout_until INTEGER NOT NULL,
    reason TEXT,
    PRIMARY KEY (community_id, pseudonym_key_hex),
    FOREIGN KEY (community_id, pseudonym_key_hex)
        REFERENCES server_members(community_id, pseudonym_key_hex) ON DELETE CASCADE
);
";
