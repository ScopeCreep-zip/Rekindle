use crate::capnp_codec;
use crate::dht::DHTManager;
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
}

/// The entire friend list stored in a single DHT record subkey.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FriendList {
    pub friends: Vec<FriendEntry>,
}

/// Create a new friend list DHT record.
///
/// Returns `(record_key, owner_keypair)`. The keypair must be persisted to retain
/// write access across sessions.
pub async fn create_friend_list(
    dht: &DHTManager,
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    let (key, owner_keypair) = dht.create_record(1).await?;

    let data = capnp_codec::friend::encode_friend_list(&[]);
    dht.set_value(&key, 0, data).await?;

    tracing::info!(key = %key, "friend list record created");
    Ok((key, owner_keypair))
}

/// Read the full friend list from DHT.
pub async fn read_friend_list(dht: &DHTManager, key: &str) -> Result<FriendList, ProtocolError> {
    match dht.get_value(key, 0).await? {
        Some(data) => {
            let friends = capnp_codec::friend::decode_friend_list(&data)?;
            Ok(FriendList { friends })
        }
        None => Ok(FriendList::default()),
    }
}

/// Add a friend to the DHT friend list.
pub async fn add_friend(
    dht: &DHTManager,
    key: &str,
    entry: FriendEntry,
) -> Result<(), ProtocolError> {
    let mut list = read_friend_list(dht, key).await?;

    // Avoid duplicates
    if list
        .friends
        .iter()
        .any(|f| f.public_key == entry.public_key)
    {
        return Ok(());
    }

    list.friends.push(entry);
    let data = capnp_codec::friend::encode_friend_list(&list.friends);
    dht.set_value(key, 0, data).await?;

    Ok(())
}

/// Remove a friend from the DHT friend list.
pub async fn remove_friend(
    dht: &DHTManager,
    key: &str,
    public_key: &str,
) -> Result<(), ProtocolError> {
    let mut list = read_friend_list(dht, key).await?;
    list.friends.retain(|f| f.public_key != public_key);
    let data = capnp_codec::friend::encode_friend_list(&list.friends);
    dht.set_value(key, 0, data).await?;

    Ok(())
}

/// Update a friend's nickname or group.
pub async fn update_friend(
    dht: &DHTManager,
    key: &str,
    public_key: &str,
    nickname: Option<String>,
    group: Option<String>,
) -> Result<(), ProtocolError> {
    let mut list = read_friend_list(dht, key).await?;
    if let Some(friend) = list.friends.iter_mut().find(|f| f.public_key == public_key) {
        if nickname.is_some() {
            friend.nickname = nickname;
        }
        if group.is_some() {
            friend.group = group;
        }
    }
    let data = capnp_codec::friend::encode_friend_list(&list.friends);
    dht.set_value(key, 0, data).await?;

    Ok(())
}
