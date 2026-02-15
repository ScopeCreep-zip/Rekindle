use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rusqlite::params;

use crate::server_state::ServerState;

/// Generate the initial MEK when a community is first hosted.
pub fn create_initial_mek(
    state: &Arc<ServerState>,
    community_id: &str,
) -> MediaEncryptionKey {
    let mek = MediaEncryptionKey::generate(1);

    let db = state.db.lock().unwrap_or_else(|e| {
        tracing::error!(error = %e, "server db mutex poisoned — recovering");
        e.into_inner()
    });
    if let Err(e) = db.execute(
        "INSERT OR REPLACE INTO server_mek (community_id, generation, key_bytes, created_at) VALUES (?,?,?,?)",
        params![community_id, 1i64, mek.as_bytes().as_slice(), timestamp_now()],
    ) {
        tracing::error!(error = %e, community = %community_id, "failed to persist initial MEK to DB");
    }

    tracing::info!(community = %community_id, "created initial MEK (generation 1)");
    mek
}

/// Rotate the MEK: generate a new key with the next generation.
///
/// Returns the new MEK. The caller is responsible for distributing it
/// to remaining members and updating the DHT.
pub fn rotate_mek(
    state: &Arc<ServerState>,
    community_id: &str,
    new_generation: u64,
) -> MediaEncryptionKey {
    let mek = MediaEncryptionKey::generate(new_generation);

    let db = state.db.lock().unwrap_or_else(|e| {
        tracing::error!(error = %e, "server db mutex poisoned — recovering");
        e.into_inner()
    });
    if let Err(e) = db.execute(
        "INSERT INTO server_mek (community_id, generation, key_bytes, created_at) VALUES (?,?,?,?)",
        params![
            community_id,
            i64::try_from(new_generation).unwrap_or(i64::MAX),
            mek.as_bytes().as_slice(),
            timestamp_now()
        ],
    ) {
        tracing::error!(error = %e, community = %community_id, generation = new_generation, "failed to persist rotated MEK to DB");
    }

    tracing::info!(
        community = %community_id,
        generation = new_generation,
        "MEK rotated"
    );
    mek
}

/// Load the latest MEK for a community from the server database.
pub fn load_latest_mek(
    state: &Arc<ServerState>,
    community_id: &str,
) -> Option<MediaEncryptionKey> {
    let db = state.db.lock().unwrap_or_else(|e| {
        tracing::error!(error = %e, "server db mutex poisoned — recovering");
        e.into_inner()
    });
    let result = db.query_row(
        "SELECT generation, key_bytes FROM server_mek WHERE community_id = ? ORDER BY generation DESC LIMIT 1",
        params![community_id],
        |row| {
            let gen: i64 = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            Ok((gen, bytes))
        },
    );

    match result {
        Ok((gen, bytes)) => {
            if bytes.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                Some(MediaEncryptionKey::from_bytes(key, gen.try_into().unwrap_or(0u64)))
            } else {
                tracing::error!(community = %community_id, "MEK key_bytes has wrong length");
                None
            }
        }
        Err(_) => None,
    }
}

fn timestamp_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}
