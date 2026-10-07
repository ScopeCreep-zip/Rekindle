use crate::capnp_codec;
use veilid_core::{DHTSchema, KeyPair};

use super::parse_record_key;
use super::pool::{RecordPool, SetOutcome};
use crate::error::ProtocolError;
use serde::{Deserialize, Serialize};

/// A single entry in the friend list DHT record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendEntry {
    /// Friend's Ed25519 public key (hex-encoded).
    pub public_key: String,
    /// Local nickname override.
    pub nickname: Option<String>,
    /// Group assignment (e.g., "Work", "Gaming").
    pub group: Option<String>,
    /// Unix timestamp when added.
    pub added_at: u64,
    /// Their profile DHT record key.
    pub profile_dht_key: Option<String>,
    /// `DhtLog` spine key for the per-peer DM conversation, created
    /// during friend accept. Both peers read and write it.
    #[serde(default)]
    pub dm_log_key: Option<String>,
}

/// The entire friend list stored in a single DHT record subkey.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FriendList {
    pub friends: Vec<FriendEntry>,
}

/// Create our friend-list record (a fresh owner key), write an empty list,
/// and hold it writable for the session. Returns the record key, its owner
/// keypair (which the caller must persist), and how the write went.
///
/// # Errors
/// The record could not be created, or the write failed outright.
pub async fn create_friend_list(
    pool: &RecordPool,
) -> Result<(String, KeyPair, SetOutcome), ProtocolError> {
    let schema = DHTSchema::dflt(1)
        .map_err(|e| ProtocolError::DhtError(format!("invalid friend list schema: {e}")))?;
    // The session keeps this lease; the pool closes it at logout.
    let (_lease, key, keypair) = pool.create(schema, None).await?;
    let key = key.to_string();
    let outcome = write_friend_list(pool, &key, &[]).await?;
    tracing::info!(key = %key, ?outcome, "friend list record created");
    Ok((key, keypair, outcome))
}

/// Hold our existing friend-list record writable for the session.
///
/// # Errors
/// The record could not be opened within the pool's retry budget. That is
/// login's to report: re-creating would orphan the list.
pub async fn open_friend_list(
    pool: &RecordPool,
    key: &str,
    owner_keypair: KeyPair,
) -> Result<(), ProtocolError> {
    // The session keeps this lease; the pool closes it at logout.
    let _lease = pool
        .acquire(&parse_record_key(key)?, Some(owner_keypair))
        .await?;
    tracing::debug!(key, "friend list record reopened");
    Ok(())
}

/// Read the full friend list.
///
/// # Errors
/// The record could not be reached, or its value does not decode.
pub async fn read_friend_list(pool: &RecordPool, key: &str) -> Result<FriendList, ProtocolError> {
    let lease = pool.acquire(&parse_record_key(key)?, None).await?;
    let value = pool.get(lease, 0, false).await;
    pool.release(lease).await;
    match value? {
        Some(data) => {
            let friends = capnp_codec::friend::decode_friend_list(data.data())?;
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
    pool: &RecordPool,
    key: &str,
    friends: &[FriendEntry],
) -> Result<SetOutcome, ProtocolError> {
    let data = capnp_codec::friend::encode_friend_list(friends);
    let lease = pool.acquire(&parse_record_key(key)?, None).await?;
    let outcome = pool.set_durable(lease, 0, data).await;
    pool.release(lease).await;
    outcome
}

/// Add a friend to our friend list (no-op when already listed).
///
/// # Errors
/// As [`read_friend_list`] and [`write_friend_list`].
pub async fn add_friend(
    pool: &RecordPool,
    key: &str,
    entry: FriendEntry,
) -> Result<SetOutcome, ProtocolError> {
    let mut list = read_friend_list(pool, key).await?;
    if list
        .friends
        .iter()
        .any(|f| f.public_key == entry.public_key)
    {
        return Ok(SetOutcome::Landed);
    }
    list.friends.push(entry);
    write_friend_list(pool, key, &list.friends).await
}

/// Remove a friend from our friend list.
///
/// # Errors
/// As [`read_friend_list`] and [`write_friend_list`].
pub async fn remove_friend(
    pool: &RecordPool,
    key: &str,
    public_key: &str,
) -> Result<SetOutcome, ProtocolError> {
    let mut list = read_friend_list(pool, key).await?;
    list.friends.retain(|f| f.public_key != public_key);
    write_friend_list(pool, key, &list.friends).await
}

/// Change a friend's local nickname and/or group in our friend list. A
/// `None` leaves that field alone. The nickname command
/// (`friend_runtime/nickname.rs`) is the caller.
///
/// # Errors
/// As [`read_friend_list`] and [`write_friend_list`].
pub async fn update_friend(
    pool: &RecordPool,
    key: &str,
    public_key: &str,
    nickname: Option<String>,
    group: Option<String>,
) -> Result<SetOutcome, ProtocolError> {
    let mut list = read_friend_list(pool, key).await?;
    if let Some(friend) = list.friends.iter_mut().find(|f| f.public_key == public_key) {
        if nickname.is_some() {
            friend.nickname = nickname;
        }
        if group.is_some() {
            friend.group = group;
        }
    }
    write_friend_list(pool, key, &list.friends).await
}
