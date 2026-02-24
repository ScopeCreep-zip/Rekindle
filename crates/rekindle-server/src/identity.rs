use std::sync::{Arc, Mutex};

use rekindle_crypto::identity::Identity;
use rusqlite::{Connection, params};

/// Load or create the server's Ed25519 identity keypair.
///
/// On first run, generates a new keypair and persists it to the `server_identity` table.
/// On subsequent runs, loads the existing keypair from the database.
pub fn load_or_create_identity(db: &Arc<Mutex<Connection>>) -> Result<(Identity, String), String> {
    let db = crate::db_helpers::lock_db(db);

    // Try to load existing identity
    let existing: Option<(String, String)> = db
        .query_row(
            "SELECT secret_key_hex, public_key_hex FROM server_identity WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    if let Some((secret_hex, public_hex)) = existing {
        let secret_bytes =
            hex::decode(&secret_hex).map_err(|e| format!("invalid secret key hex: {e}"))?;
        let secret_array: [u8; 32] = secret_bytes
            .try_into()
            .map_err(|_| "secret key must be 32 bytes".to_string())?;
        let identity = Identity::from_secret_bytes(&secret_array);

        tracing::info!(public_key = %public_hex, "loaded server identity from DB");
        return Ok((identity, public_hex));
    }

    // Generate new identity
    let identity = Identity::generate();
    let secret_hex = hex::encode(identity.secret_key_bytes());
    let public_hex = hex::encode(identity.public_key_bytes());
    let now = rekindle_utils::timestamp_secs_i64();

    db.execute(
        "INSERT INTO server_identity (id, secret_key_hex, public_key_hex, created_at) VALUES (1, ?, ?, ?)",
        params![secret_hex, public_hex, now],
    )
    .map_err(|e| format!("failed to persist server identity: {e}"))?;

    tracing::info!(public_key = %public_hex, "generated new server identity");
    Ok((identity, public_hex))
}
