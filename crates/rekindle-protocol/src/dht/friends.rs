use rekindle_codec::capnp_codec;
use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, SetOutcome};

use crate::error::ProtocolError;
use rekindle_codec::friend::FriendEntry;
use serde::{Deserialize, Serialize};

/// The entire friend list stored in a single DHT record subkey.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FriendList {
    pub friends: Vec<FriendEntry>,
}

/// Create our friend-list record (a fresh owner key), write an empty list,
/// and hold it writable for the session. Returns the session lease, the
/// record key, its owner keypair (which the caller must persist), and how
/// the write went.
///
/// # Errors
/// The record could not be created, or the write failed outright.
pub async fn create_friend_list(
    pool: &dyn RecordPort,
) -> Result<(LeaseId, String, String, SetOutcome), ProtocolError> {
    let (lease, key, keypair) = pool
        .create(&rekindle_records::schema::single(), None)
        .await?;
    let outcome = write_friend_list(pool, &key, &[]).await?;
    tracing::info!(key = %key, ?outcome, "friend list record created");
    Ok((lease, key, keypair, outcome))
}

/// Hold our existing friend-list record writable for the session. Returns
/// the session lease.
///
/// # Errors
/// The record could not be opened within the pool's retry budget. That is
/// login's to report: re-creating would orphan the list.
pub async fn open_friend_list(
    pool: &dyn RecordPort,
    key: &str,
    owner_keypair: &str,
) -> Result<LeaseId, ProtocolError> {
    let lease = pool.acquire(key, Some(owner_keypair)).await?;
    tracing::debug!(key, "friend list record reopened");
    Ok(lease)
}

/// Read the full friend list.
///
/// # Errors
/// The record could not be reached, or its value does not decode.
pub async fn read_friend_list(
    pool: &dyn RecordPort,
    key: &str,
) -> Result<FriendList, ProtocolError> {
    match pool.read_once(key, 0, false).await? {
        Some(value) => {
            let friends = capnp_codec::friend::decode_friend_list(&value.data)?;
            Ok(FriendList { friends })
        }
        None => Ok(FriendList::default()),
    }
}

/// Replace our friend list. Needs the session's writable lease (taken at
/// login by [`create_friend_list`] or [`open_friend_list`]).
///
/// # Errors
/// The record could not be reached, or the write failed outright.
pub async fn write_friend_list(
    pool: &dyn RecordPort,
    key: &str,
    friends: &[FriendEntry],
) -> Result<SetOutcome, ProtocolError> {
    let data = capnp_codec::friend::encode_friend_list(friends);
    let lease = pool.acquire(key, None).await?;
    let outcome = pool.set_durable(lease, 0, data, None).await;
    pool.release(lease).await;
    Ok(outcome?)
}
