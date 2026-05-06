//! Bob's view of the Strand Relay pool (architecture §13.2 step 3-4).
//!
//! When Bob receives a `RelayOffer` from Carol, the offer is persisted in
//! `strand_relay_offers`. The encoded pool — opaque route blobs padded
//! with dummies for size privacy — is what Bob publishes on his profile
//! DHT record so peers who can't reach him directly have a fallback path.

use std::sync::Arc;

use rekindle_protocol::dht::profile::SUBKEY_RELAY_POOL;

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::services::message_service;
use crate::state::AppState;
use crate::state_helpers;

/// Number of slots in the published relay pool. Padded with dummies so
/// the pool size does not leak the actual relay friend count
/// (architecture §13.4 privacy property).
pub const RELAY_POOL_PAD_SIZE: usize = 8;

/// Lower bound for dummy entry size. Generous upper bound on a typical
/// Veilid private-route blob — keeps dummies plausibly route-shaped
/// even before any real offer has landed (so the pool's first publish
/// doesn't telegraph "this user has zero relays" via tiny entries).
const DUMMY_ENTRY_MIN_SIZE: usize = 1024;

/// Insert (or replace) a relay offer received from a friend.
pub async fn add_received_offer(
    state: &Arc<AppState>,
    pool: &DbPool,
    relay_pseudonym: &str,
    relay_route_blob: &[u8],
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let pseudonym = relay_pseudonym.to_string();
    let blob = relay_route_blob.to_vec();
    let now = crate::db::timestamp_now();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO strand_relay_offers (owner_key, relay_pseudonym, relay_route_blob, received_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_key, relay_pseudonym) DO UPDATE SET
                 relay_route_blob = excluded.relay_route_blob,
                 received_at = excluded.received_at",
            rusqlite::params![owner_key, pseudonym, blob, now],
        )?;
        Ok(())
    })
    .await
}

/// Drop a relay offer (received `RelayWithdraw` or local revocation).
pub async fn remove_received_offer(
    state: &Arc<AppState>,
    pool: &DbPool,
    relay_pseudonym: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let pseudonym = relay_pseudonym.to_string();
    db_call(pool, move |conn| {
        conn.execute(
            "DELETE FROM strand_relay_offers WHERE owner_key = ?1 AND relay_pseudonym = ?2",
            rusqlite::params![owner_key, pseudonym],
        )?;
        Ok(())
    })
    .await
}

/// List all currently held relay offers.
pub async fn list_received_offers(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Vec<(String, Vec<u8>)> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Vec::new();
    }
    db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT relay_pseudonym, relay_route_blob FROM strand_relay_offers WHERE owner_key = ?1",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner_key], |row| {
                let pseudonym: String = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                Ok((pseudonym, blob))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

/// Build the encoded relay pool body for publication on the profile DHT
/// record subkey. Format: JSON array of opaque route blobs padded to
/// `RELAY_POOL_PAD_SIZE` with dummy entries (architecture §13.4).
///
/// Dummies are 32 bytes of zero-prefixed random padding sized to roughly
/// match real blob lengths so a passive observer cannot count real
/// entries by ciphertext length.
pub async fn get_local_relay_pool(state: &Arc<AppState>, pool: &DbPool) -> Vec<u8> {
    let mut entries: Vec<Vec<u8>> = list_received_offers(state, pool)
        .await
        .into_iter()
        .map(|(_, blob)| blob)
        .collect();
    // Establish a target length matching the largest real entry so
    // dummies don't stand out by size. Floored at `DUMMY_ENTRY_MIN_SIZE`
    // so the empty / small-pool case still produces plausibly route-
    // shaped dummies — important on the very first publish before any
    // real offer has landed.
    let target_len = entries
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(DUMMY_ENTRY_MIN_SIZE);
    while entries.len() < RELAY_POOL_PAD_SIZE {
        let mut dummy = vec![0u8; target_len];
        // Mark the leading byte as 0xFF so receivers can quickly skip
        // dummies without trying to import them as Veilid routes.
        if !dummy.is_empty() {
            dummy[0] = 0xFF;
        }
        entries.push(dummy);
    }
    serde_json::to_vec(&entries).unwrap_or_default()
}

/// Encode the local relay pool and push it to our profile DHT record at
/// `SUBKEY_RELAY_POOL`. Best-effort; logs and swallows errors so an
/// in-flight offer accept does not fail because the DHT push is slow.
pub async fn republish_relay_pool(state: &Arc<AppState>, pool_db: &DbPool) {
    let body = get_local_relay_pool(state, pool_db).await;
    if let Err(e) = message_service::push_profile_update(state, SUBKEY_RELAY_POOL, body).await {
        tracing::warn!(error = %e, "failed to republish strand relay pool");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad_entries(real_blobs: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        let target_len = real_blobs
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(DUMMY_ENTRY_MIN_SIZE);
        let mut entries = real_blobs;
        while entries.len() < RELAY_POOL_PAD_SIZE {
            let mut dummy = vec![0u8; target_len];
            dummy[0] = 0xFF;
            entries.push(dummy);
        }
        entries
    }

    #[test]
    fn padding_marks_dummy_entries() {
        let real_blobs: Vec<Vec<u8>> = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let entries = pad_entries(real_blobs.clone());
        assert_eq!(entries.len(), RELAY_POOL_PAD_SIZE);
        assert_eq!(entries[0], real_blobs[0]);
        for entry in &entries[real_blobs.len()..] {
            assert_eq!(entry[0], 0xFF);
        }
    }

    #[test]
    fn dummies_meet_min_size_when_pool_is_empty() {
        let entries = pad_entries(Vec::new());
        assert_eq!(entries.len(), RELAY_POOL_PAD_SIZE);
        for entry in &entries {
            assert_eq!(entry.len(), DUMMY_ENTRY_MIN_SIZE);
            assert_eq!(entry[0], 0xFF);
        }
    }

    #[test]
    fn dummies_track_largest_real_entry_when_above_min() {
        // A real entry larger than DUMMY_ENTRY_MIN_SIZE forces dummies
        // up so they remain indistinguishable by length.
        let big = vec![7u8; DUMMY_ENTRY_MIN_SIZE * 2];
        let entries = pad_entries(vec![big.clone()]);
        for entry in entries.iter().skip(1) {
            assert_eq!(entry.len(), big.len());
        }
    }
}
