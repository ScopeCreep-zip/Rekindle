use rekindle_records::lease::LeaseId;
use veilid_core::{DHTSchema, KeyPair};

use super::parse_record_key;
use super::pool::{RecordPool, SetOutcome};
use crate::error::ProtocolError;

// Subkey constants for the user profile DHT record.
//
// The layout itself lives in `rekindle_types::dht_layout::profile` —
// the daemon track indexes the same records, and keeping a private copy
// here is how the two ended up disagreeing about subkey 8. These are
// aliases so existing call sites keep reading naturally.
pub use rekindle_types::dht_layout::profile::{
    AVATAR as SUBKEY_AVATAR, DISPLAY_NAME as SUBKEY_DISPLAY_NAME, GAME_INFO as SUBKEY_GAME_INFO,
    METADATA as SUBKEY_METADATA, PREKEY_BUNDLE as SUBKEY_PREKEY_BUNDLE,
    RELAY_POOL as SUBKEY_RELAY_POOL, ROUTE_BLOB as SUBKEY_ROUTE_BLOB, STATUS as SUBKEY_STATUS,
    STATUS_MESSAGE as SUBKEY_STATUS_MESSAGE, STATUS_ONLINE, SUBKEY_COUNT as PROFILE_SUBKEY_COUNT,
};

/// What login publishes to our profile record.
#[derive(Debug, Clone, Copy)]
pub struct ProfileFields<'a> {
    pub display_name: &'a str,
    pub status_message: &'a str,
    pub prekey_bundle: &'a [u8],
    pub route_blob: &'a [u8],
}

/// Create our profile record (a fresh owner key), publish `fields`, and
/// hold it writable for the session. Returns the session lease (released
/// when the profile is rotated away, else at logout), the record key, its
/// owner keypair (which the caller must persist), and how the writes went.
///
/// The key is random per create (V4), so a profile is created once, when
/// the identity has none, and re-opened by [`open_profile`] on every later
/// login.
///
/// # Errors
/// The record could not be created, or a write failed outright.
pub async fn create_profile(
    pool: &RecordPool,
    fields: ProfileFields<'_>,
) -> Result<(LeaseId, String, KeyPair, SetOutcome), ProtocolError> {
    let schema = DHTSchema::dflt(
        u16::try_from(PROFILE_SUBKEY_COUNT)
            .map_err(|e| ProtocolError::DhtError(format!("profile subkey count: {e}")))?,
    )
    .map_err(|e| ProtocolError::DhtError(format!("invalid profile schema: {e}")))?;
    let (lease, key, keypair) = pool.create(schema, None).await?;
    let key = key.to_string();
    let outcome = publish_fields(pool, &key, fields, "profile create").await?;
    tracing::info!(key = %key, name = %fields.display_name, ?outcome, "profile record created");
    Ok((lease, key, keypair, outcome))
}

/// Hold our existing profile record writable for the session. Returns the
/// session lease.
///
/// # Errors
/// The record could not be opened within the pool's retry budget. That is
/// login's to report: re-creating would change the key peers know.
pub async fn open_profile(
    pool: &RecordPool,
    key: &str,
    owner_keypair: KeyPair,
) -> Result<LeaseId, ProtocolError> {
    let lease = pool
        .acquire(&parse_record_key(key)?, Some(owner_keypair))
        .await?;
    tracing::info!(key, "profile record reopened");
    Ok(lease)
}

/// Publish the login fields to our profile (needs the session lease from
/// [`open_profile`]). Returns the first missed outcome
/// ([`SetOutcome::missed`]), or `Landed`.
///
/// # Errors
/// The record could not be reached, or a write failed outright.
pub async fn publish_profile_fields(
    pool: &RecordPool,
    key: &str,
    fields: ProfileFields<'_>,
) -> Result<SetOutcome, ProtocolError> {
    publish_fields(pool, key, fields, "profile reopen").await
}

/// Write one subkey of our own profile. Needs the session's writable lease
/// (taken at login by [`create_profile`] or [`open_profile`]): the write is
/// signed by that lease's writer, and fails if there is none. Durable: a
/// write that misses consensus is held and re-pushed by the pool until it
/// lands (`RecordPool::set_durable`).
///
/// # Errors
/// The record could not be reached, or the write failed outright.
pub async fn set_own_profile_subkey(
    pool: &RecordPool,
    key: &str,
    subkey: u32,
    value: Vec<u8>,
) -> Result<SetOutcome, ProtocolError> {
    let lease = pool.acquire(&parse_record_key(key)?, None).await?;
    let outcome = pool.set_durable(lease, subkey, value).await;
    pool.release(lease).await;
    outcome
}

/// Write our own STATUS subkey (status byte + time). Present-tense: a plain
/// write, never held for re-push, because a status landing late would tell
/// readers we were reachable when we were not (`pool/durable.rs`); the 120 s
/// heartbeat writes it again. Needs the session's writable profile lease.
///
/// # Errors
/// The record could not be reached, or the write failed outright.
pub async fn set_own_profile_status(
    pool: &RecordPool,
    key: &str,
    value: Vec<u8>,
) -> Result<SetOutcome, ProtocolError> {
    let lease = pool.acquire(&parse_record_key(key)?, None).await?;
    let outcome = pool.set(lease, SUBKEY_STATUS, value, None).await;
    pool.release(lease).await;
    outcome
}

/// One subkey of a profile, or `None` if it has not been published.
/// `force_refresh` asks the network rather than the local copy.
///
/// # Errors
/// The record could not be opened or read.
pub async fn read_profile_subkey(
    pool: &RecordPool,
    key: &str,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    let lease = pool.acquire(&parse_record_key(key)?, None).await?;
    let value = pool.get(lease, subkey, force_refresh).await;
    pool.release(lease).await;
    Ok(value?.map(|v| v.data().to_vec()))
}

/// The status subkey's payload: the status byte, then the time in ms.
#[must_use]
pub fn status_payload(status: u8) -> Vec<u8> {
    let mut payload = Vec::with_capacity(9);
    payload.push(status);
    payload.extend_from_slice(&rekindle_utils::timestamp_ms_i64().to_be_bytes());
    payload
}

/// Write the login fields (display name, status message, prekey bundle,
/// route blob), all durable. Not the STATUS: the session's one STATUS
/// publisher writes it once the profile is open, so login never writes a
/// status over one the user already picked (plan C7.8c). Returns the first
/// missed outcome ([`SetOutcome::missed`]), or `Landed`.
async fn publish_fields(
    pool: &RecordPool,
    key: &str,
    fields: ProfileFields<'_>,
    context: &str,
) -> Result<SetOutcome, ProtocolError> {
    let writes = [
        (SUBKEY_DISPLAY_NAME, fields.display_name.as_bytes().to_vec()),
        (
            SUBKEY_STATUS_MESSAGE,
            fields.status_message.as_bytes().to_vec(),
        ),
        (SUBKEY_PREKEY_BUNDLE, fields.prekey_bundle.to_vec()),
        (SUBKEY_ROUTE_BLOB, fields.route_blob.to_vec()),
    ];
    let mut first_miss = SetOutcome::Landed;
    for (subkey, value) in writes {
        let outcome = set_own_profile_subkey(pool, key, subkey, value).await?;
        if subkey == SUBKEY_PREKEY_BUNDLE {
            tracing::info!(
                subkey,
                bytes = fields.prekey_bundle.len(),
                ?outcome,
                "pqxdh_bundle_published kind=LastResort+OneTimeBatch ({context})",
            );
        }
        if outcome.missed() {
            if matches!(outcome, SetOutcome::Superseded(_)) {
                // A newer write of ours (another device of this identity) won.
                tracing::info!(
                    key,
                    subkey,
                    "profile subkey already newer on the network ({context})"
                );
            } else {
                tracing::warn!(
                    key,
                    subkey,
                    ?outcome,
                    "profile subkey not stored at consensus ({context})"
                );
            }
            if !first_miss.missed() {
                first_miss = outcome;
            }
        }
    }
    Ok(first_miss)
}

// The five pull accessors that lived here — `read_subkey`,
// `read_display_name`, `read_status`, `read_route_blob`,
// `read_prekey_bundle` — are gone. Nothing pulled a peer's profile
// subkey on demand: display name, status and route blob arrive through
// the presence watch, and a peer's prekey bundle arrives in the friend
// request or invite payload that needs it. Push won; the pull half was
// never wired to anything. (`read_profile_subkey` above serves the daemon's
// friend-request flow and its friend-list queries.)
