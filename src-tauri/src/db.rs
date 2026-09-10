use rusqlite::Connection;

/// Async database handle backed by a dedicated background thread.
///
/// [`tokio_rusqlite::Connection`] wraps a single [`rusqlite::Connection`] on a
/// background thread and exposes an async `call()` API.  It is Clone + Send
/// + Sync, so Tauri's `State<'_, DbPool>` works out of the box.
pub type DbPool = tokio_rusqlite::Connection;

/// Bump this every time `001_init.sql` changes, or when a DHT record
/// layout is renumbered — the reset also wipes Veilid storage, so stale
/// records written under the old layout cannot be read back with the
/// new indices.  On mismatch the entire database is wiped and recreated
/// from the schema — safe because the app is not live yet and identity
/// keys live in Stronghold, not `SQLite`.
///
/// 81: the friend-list DHT record. `FriendEntry` gained `dm_log_key`
/// and `profileDhtKey`/`dmLogKey` joined `schemas/friend.capnp`, which
/// had only four fields while the Rust struct had five — the encoder
/// dropped `profile_dht_key` on every write and the decoder set it to
/// `None` with a comment saying so. The daemon track also wrote this
/// record in postcard while the desktop wrote Cap'n Proto, so a list
/// written by either is unreadable to the other.
///
/// 80: the account record's header shed the three child
/// `DHTShortArray` pointers and their owner keypairs. Creating an
/// account allocated a contact list, a chat list and an invitation list
/// that nothing ever wrote an entry to, and every login reopened all
/// three. `AccountHeader` is now the five fields it actually carries,
/// so a header written under the old schema decodes its text fields at
/// the wrong ordinals.
///
/// 79: the session membership record lost `is_operator`,
/// `community_mailbox_key` and `join_inbox_key`. All three were the
/// coordinator's, and the mailbox they named was never created by
/// anything — `create_community_mailbox` had no callers, so the route
/// the authority loop published into it went nowhere.
///
/// 76: `MemberPresence` gained `departed`, the signed tombstone a
/// leaver writes into its own registry slot so the slot can be reused.
/// Live rows are unaffected — the field is `skip_serializing_if`, so a
/// non-departed row stays byte-identical and old readers still verify
/// it. Only a departed row carries it, and an old reader drops the
/// unknown field before recomputing `signing_bytes()`, fails the
/// signature, and treats the slot as reclaimable too: both ends reach
/// the same answer. Bumped because the wire format grew a field, not
/// because the tracks disagree.
///
/// 75: the registry MEK vault is gone from community creation, and the
/// creator no longer writes itself into a shared member index. Both were
/// owner-subkey writes that `o_cnt: 0` grants nobody a credential for,
/// and `communities-channels.md` says the MEK is *never* written to DHT.
/// A community created under the old flow therefore carries a vault and
/// an index entry that new peers neither write nor read, while its
/// genesis MEK sits on the DHT where it does not belong. Rotation is now
/// peer-to-peer for both shells (`rekindle-mek-rotation`), delivered by
/// `app_call` rather than published.
///
/// 74: admission governance entries. `GovernanceEntry` gained
/// `JoinRequested`, `MemberApproved`, `MemberRejected` and
/// `AdmissionPolicy` (Cap'n Proto union arms 34-37), so a peer on the
/// old schema cannot read entries written by a new one — the union
/// discriminant is unknown to it. Also flips the daemon to the v2.0
/// self-sovereign join: members claim their own registry slot from the
/// invite's shared slot seed instead of being assigned one, so slots
/// written under the old flow used a locally-derived seed that no longer
/// resolves.
///
/// 73: profile DHT record renumbered. The two tracks had disagreed about
/// subkey 8 (Strand Relay pool vs friend-inbox key); the friend-inbox
/// pair moved to 9/10 and the record now allocates 11 subkeys. See
/// `rekindle_types::dht_layout::profile`.
/// 77: the member registry lost its last community-wide subkeys. The
/// member index (0), MEK vault (1) and moderation queue (5) were v1.0
/// owner subkeys that `o_cnt: 0` credentials nobody to write, and each
/// collided with a real member slot. Membership is now derived by
/// scanning signed presence rows, the MEK is delivered peer-to-peer and
/// never written to the DHT, and admission is a `GovernanceEntry`.
///
/// Landing with it: the daemon's channel storage moved from a
/// per-member DFLT `DhtLog` to the SMPL `(channel, segment)` records the
/// desktop already wrote, so the two tracks finally read and write the
/// same messages. Discovery is `ChannelCreated.record_key` plus
/// `ChannelSegmentLinked` out of merged governance, which is what made
/// the member index deletable.
/// 78: genesis entries gained `AdmissionPolicy` at lamport 1, shifting
/// the other five up by one. Every community created before this has an
/// entry set that cannot express `ApprovalRequired` — the variant was
/// merged, validated and Cap'n Proto encoded, and nothing ever wrote it,
/// so the mode was unreachable and every community was `Open`.
const SCHEMA_VERSION: i64 = 82;

/// Result of opening the database — includes a flag indicating whether the
/// schema was recreated from scratch (so the caller can wipe dependent storage).
pub struct DbOpenResult {
    pub pool: DbPool,
    /// `true` when the schema version changed and all tables were dropped and
    /// recreated.  The caller should wipe Stronghold files and Veilid storage
    /// to avoid orphaned state.
    pub schema_reset: bool,
}

/// Open (or create) a `SQLite` database at `db_path` and run the initial schema
/// migration.  Returns a `DbOpenResult` with the pool and a reset flag.
///
/// The raw `rusqlite::Connection` is created and configured synchronously
/// (PRAGMAs, schema check), then wrapped in `tokio_rusqlite::Connection`
/// which spawns a dedicated background thread for all future DB access.
pub fn create_pool(db_path: &str) -> Result<DbOpenResult, String> {
    let conn =
        Connection::open(db_path).map_err(|e| format!("failed to connect to database: {e}"))?;

    // Enable WAL mode for better concurrent-read performance.
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| format!("failed to set WAL mode: {e}"))?;

    // Enable foreign key constraint enforcement (off by default in SQLite).
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("failed to enable foreign keys: {e}"))?;

    // Check schema version — wipe and recreate if stale.
    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);

    let schema_reset = current != SCHEMA_VERSION;

    if schema_reset {
        if current != 0 {
            tracing::info!(
                old = current,
                new = SCHEMA_VERSION,
                "schema version mismatch — recreating database"
            );
        }
        drop_all_tables(&conn)?;
        conn.execute_batch(include_str!("../migrations/001_init.sql"))
            .map_err(|e| format!("failed to run schema: {e}"))?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| format!("failed to set schema version: {e}"))?;
    }

    // Wrap configured connection — spawns the background thread.
    Ok(DbOpenResult {
        pool: tokio_rusqlite::Connection::from(conn),
        schema_reset,
    })
}

/// Drop every user table so the schema can be cleanly re-applied.
///
/// Virtual tables (FTS5) must be dropped before regular tables — they own
/// shadow tables (`<name>_data`, `<name>_idx`, etc.) which SQLite refuses
/// to drop directly. We identify them via `sql LIKE 'CREATE VIRTUAL%'`
/// and skip rows whose `sql` is NULL (shadow tables).
fn drop_all_tables(conn: &Connection) -> Result<(), String> {
    // Must disable FK checks while dropping to avoid ordering issues.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| format!("failed to disable foreign keys: {e}"))?;

    let virtual_tables = list_tables_matching(
        conn,
        "type='table' AND sql LIKE 'CREATE VIRTUAL%' AND name NOT LIKE 'sqlite_%'",
    )?;
    for table in &virtual_tables {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\";"))
            .map_err(|e| format!("failed to drop virtual table {table}: {e}"))?;
    }

    let real_tables = list_tables_matching(
        conn,
        "type='table' AND sql LIKE 'CREATE TABLE%' AND name NOT LIKE 'sqlite_%'",
    )?;
    for table in &real_tables {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\";"))
            .map_err(|e| format!("failed to drop table {table}: {e}"))?;
    }

    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("failed to re-enable foreign keys: {e}"))?;

    Ok(())
}

fn list_tables_matching(conn: &Connection, where_clause: &str) -> Result<Vec<String>, String> {
    let sql = format!("SELECT name FROM sqlite_master WHERE {where_clause}");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("failed to list tables: {e}"))?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("failed to query tables: {e}"))?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(names)
}

/// Extract a `String` column by name, returning `""` on any failure.
pub fn get_str(row: &rusqlite::Row<'_>, col: &str) -> String {
    row.get::<_, String>(col).unwrap_or_default()
}

/// Extract an optional `String` column by name.
pub fn get_str_opt(row: &rusqlite::Row<'_>, col: &str) -> Option<String> {
    row.get::<_, Option<String>>(col).ok().flatten()
}

/// Extract an `i64` column by name, returning `0` on any failure.
pub fn get_i64(row: &rusqlite::Row<'_>, col: &str) -> i64 {
    row.get::<_, i64>(col).unwrap_or_default()
}

/// Current UNIX timestamp in milliseconds.
pub fn timestamp_now() -> i64 {
    rekindle_utils::timestamp_ms_i64()
}
