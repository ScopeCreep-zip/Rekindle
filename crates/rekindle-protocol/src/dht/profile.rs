use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, SetOutcome};

use crate::error::ProtocolError;

// Subkey constants for the user profile DHT record.
//
// The layout itself lives in `rekindle_types::dht_layout::profile` —
// the daemon track indexes the same records, and keeping a private copy
// here is how the two ended up disagreeing about subkey 8. These are
// aliases so existing call sites keep reading naturally.
pub use rekindle_types::dht_layout::profile::{
    AVATAR as SUBKEY_AVATAR, DISPLAY_NAME as SUBKEY_DISPLAY_NAME, GAME_INFO as SUBKEY_GAME_INFO,
    METADATA as SUBKEY_METADATA, PREKEY_RECORDS as SUBKEY_PREKEY_RECORDS,
    RELAY_POOL as SUBKEY_RELAY_POOL, ROUTE_BLOB as SUBKEY_ROUTE_BLOB, STATUS as SUBKEY_STATUS,
    STATUS_MESSAGE as SUBKEY_STATUS_MESSAGE, STATUS_ONLINE, SUBKEY_COUNT as PROFILE_SUBKEY_COUNT,
};

/// What login publishes to our profile record.
#[derive(Debug, Clone, Copy)]
pub struct ProfileFields<'a> {
    pub display_name: &'a str,
    pub status_message: &'a str,
    /// Our device list (plan E2.4): the encoded `PrekeyRecordList`
    /// naming each device's prekey record. `None` at the unlock, which
    /// writes it with [`publish_device_list`] once the record it names
    /// holds its first-contact keys, so the rest of the profile need not
    /// wait for them (plan E2.B).
    pub prekey_records: Option<&'a [u8]>,
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
    pool: &dyn RecordPort,
    fields: ProfileFields<'_>,
) -> Result<(LeaseId, String, String, SetOutcome), ProtocolError> {
    let (lease, key, keypair) = pool
        .create(&rekindle_records::schema::profile(), None)
        .await?;
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
    pool: &dyn RecordPort,
    key: &str,
    owner_keypair: &str,
) -> Result<LeaseId, ProtocolError> {
    let lease = pool.acquire(key, Some(owner_keypair)).await?;
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
    pool: &dyn RecordPort,
    key: &str,
    fields: ProfileFields<'_>,
) -> Result<SetOutcome, ProtocolError> {
    publish_fields(pool, key, fields, "profile reopen").await
}

/// Write our device list (subkey 5, plan E2.4) to our profile: the unlock
/// calls it once the prekey record it names holds its first-contact keys.
///
/// # Errors
/// The record could not be reached, or the write failed outright.
pub async fn publish_device_list(
    pool: &dyn RecordPort,
    key: &str,
    list: &[u8],
) -> Result<SetOutcome, ProtocolError> {
    set_own_profile_subkey(pool, key, SUBKEY_PREKEY_RECORDS, list.to_vec()).await
}

/// Write one subkey of our own profile. Needs the session's writable lease
/// (taken at login by [`create_profile`] or [`open_profile`]): the write is
/// signed by that lease's writer, and fails if there is none. Durable: a
/// write that misses consensus is held and re-pushed by the pool until it
/// lands (`RecordPort::set_durable`).
///
/// # Errors
/// The record could not be reached, or the write failed outright.
pub async fn set_own_profile_subkey(
    pool: &dyn RecordPort,
    key: &str,
    subkey: u32,
    value: Vec<u8>,
) -> Result<SetOutcome, ProtocolError> {
    let lease = pool.acquire(key, None).await?;
    let outcome = pool.set_durable(lease, subkey, value, None).await;
    pool.release(lease).await;
    Ok(outcome?)
}

/// Write our own STATUS subkey (status byte + time). Present-tense: a plain
/// write, never held for re-push, because a status landing late would tell
/// readers we were reachable when we were not (`pool/durable.rs`); the 120 s
/// heartbeat writes it again. Needs the session's writable profile lease.
///
/// # Errors
/// The record could not be reached, or the write failed outright.
pub async fn set_own_profile_status(
    pool: &dyn RecordPort,
    key: &str,
    value: Vec<u8>,
) -> Result<SetOutcome, ProtocolError> {
    Ok(pool.write_once(key, SUBKEY_STATUS, value, None).await?)
}

/// One subkey of a profile, or `None` if it has not been published.
/// `force_refresh` asks the network rather than the local copy.
///
/// # Errors
/// The record could not be opened or read.
pub async fn read_profile_subkey(
    pool: &dyn RecordPort,
    key: &str,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    Ok(pool
        .read_once(key, subkey, force_refresh)
        .await?
        .map(|v| v.data))
}

/// The status subkey's payload: the status byte, then `now_ms` (the
/// caller's clock).
#[must_use]
pub fn status_payload(status: u8, now_ms: i64) -> Vec<u8> {
    let mut payload = Vec::with_capacity(9);
    payload.push(status);
    payload.extend_from_slice(&now_ms.to_be_bytes());
    payload
}

/// Write the login fields (display name, status message, prekey device list,
/// route blob), all durable. Not the STATUS: the session's one STATUS
/// publisher writes it once the profile is open, so login never writes a
/// status over one the user already picked (plan C7.8c). Returns the first
/// missed outcome ([`SetOutcome::missed`]), or `Landed`.
async fn publish_fields(
    pool: &dyn RecordPort,
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
        (SUBKEY_ROUTE_BLOB, fields.route_blob.to_vec()),
    ]
    .into_iter()
    .chain(
        fields
            .prekey_records
            .map(|list| (SUBKEY_PREKEY_RECORDS, list.to_vec())),
    );
    // The subkeys are written at once: Veilid locks per subkey, so
    // they overlap and the profile lands in one set latency (plan E2.B).
    let outcomes = futures::future::join_all(writes.map(|(subkey, value)| async move {
        (
            subkey,
            set_own_profile_subkey(pool, key, subkey, value).await,
        )
    }))
    .await;
    let mut first_miss = SetOutcome::Landed;
    for (subkey, outcome) in outcomes {
        let outcome = outcome?;
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

// A peer's profile is pushed to us through the presence watch (display
// name, status, route). The one pull is the first contact's read of the
// prekey device list (`super::prekeys::read_device_list`, plan E2.4), and
// `read_profile_subkey` above serves the friend-list queries.
