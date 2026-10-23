//! The daemon's [`Session`], projected from the unlocked identity's stores
//! (plan D1: the daemon's session file is retired).
//!
//! [`load`] builds it at unlock from the identity's database and vault:
//! the identity row and the friends (plan E2.1; pending requests are read where they are answered,
//! `repo::pending_requests`). Every
//! change after that is written to the store that holds it, then applied
//! to the projection; nothing writes the projection back as a whole.

use rekindle_db::repo;
use rekindle_db::Db;
use rekindle_secrets::friend_presence::PresenceKey;
use rekindle_transport::session::{Session, SessionIdentity};
use rekindle_types::friend::FriendState;
use rekindle_vault::typed::owner_keys::{load_owner, OwnerKey};
use rekindle_vault::VaultHandle;

/// Loading the projection failed.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("the identity database has no identity row")]
    NoIdentityRow,
    #[error("identity database: {0}")]
    Db(String),
    #[error("identity vault: {0}")]
    Vault(String),
}

/// Everything the projection reads from the database, in one call.
struct Rows {
    display_name: String,
    records: repo::identity::IdentityDhtColumns,
    friends: Vec<repo::friends::FriendRow>,
    presence_key: (u32, Vec<u8>),
    friend_presence_keys: Vec<(String, u32, Vec<u8>)>,
}

/// Build the session of the unlocked identity `owner`. Its first unlock
/// mints our friend-presence key (plan E2.5) from `rng`.
///
/// # Errors
/// The identity row is missing, or a store cannot be read.
pub async fn load(
    db: &Db,
    vault: &VaultHandle,
    owner: &str,
    rng: &dyn rekindle_utils::RandomSource,
    now_ms: i64,
) -> Result<Session, SessionError> {
    let o = owner.to_string();
    let first_key = PresenceKey::mint(1, rng);
    let rows = db
        .call(move |conn| {
            let Some((display_name, records)) = repo::identity::login_row(conn, &o)? else {
                return Ok(None);
            };
            let friends = repo::friends::load_all(conn, &o)?;
            let presence_key = if let Some(key) = repo::presence_keys::current(conn, &o)? {
                key
            } else {
                let (epoch, key) = (first_key.epoch(), first_key.as_bytes().to_vec());
                repo::presence_keys::replace(conn, &o, epoch, &key, now_ms)?;
                (epoch, key)
            };
            let friend_presence_keys = repo::friends::presence_keys(conn, &o)?;
            Ok(Some(Rows {
                display_name,
                records,
                friends,
                presence_key,
                friend_presence_keys,
            }))
        })
        .await
        .map_err(|e| SessionError::Db(e.to_string()))?
        .ok_or(SessionError::NoIdentityRow)?;

    let guard = vault.lock();
    let vault = guard
        .as_ref()
        .ok_or_else(|| SessionError::Vault("locked".into()))?;
    // A keypair only for the record the row names: an entry naming another
    // (an interrupted write) is the own-records stage's to repair, and it
    // projects the keys it holds.
    let keypair = |record, row_key: Option<&str>| {
        load_owner(vault, record)
            .map(|entry| {
                entry
                    .filter(|entry| Some(entry.record_key.as_str()) == row_key)
                    .map(|entry| entry.keypair)
            })
            .map_err(|e| SessionError::Vault(e.to_string()))
    };
    let identity = SessionIdentity {
        public_key_hex: owner.to_string(),
        display_name: rows.display_name,
        profile_owner_keypair: keypair(OwnerKey::Profile, rows.records.profile.as_deref())?,
        friend_list_owner_keypair: keypair(
            OwnerKey::FriendList,
            rows.records.friend_list.as_deref(),
        )?,
        // Unnamed until the own-records stage holds each record
        // (`OwnRecords::project`, live finding 24): a key named before its
        // record is held writable sent the route publisher and the status
        // heartbeat to write it first, and both failed "not held writable".
        profile_dht_key: String::new(),
        mailbox_dht_key: String::new(),
        friend_list_dht_key: String::new(),
    };

    // The memberships are the community store's projection, made by the
    // warm-cache stage (plan E3.1 C1, `adapters::governance::loader`).
    let mut session = Session::new(identity);
    drop(guard);
    session.friends = rows
        .friends
        .into_iter()
        .map(|row| (row.public_key.clone(), friend_state(row)))
        .collect();
    let (epoch, key) = rows.presence_key;
    session.presence_key = PresenceKey::from_stored(epoch, &key);
    session.friend_presence_keys = rows
        .friend_presence_keys
        .into_iter()
        .filter_map(|(peer, epoch, key)| Some((peer, PresenceKey::from_stored(epoch, &key)?)))
        .collect();
    Ok(session)
}

/// A stored friend as the projection holds it: offline until their
/// presence is seen, nothing unread.
fn friend_state(row: repo::friends::FriendRow) -> FriendState {
    let mut friend = FriendState::new(row.public_key, row.display_name, row.friendship_state);
    friend.nickname = row.nickname;
    friend.group = row.group_name;
    friend.dht_record_key = row.dht_record_key;
    friend.last_seen_at = row.last_seen_at;
    friend.mailbox_dht_key = row.mailbox_dht_key;
    friend
}

mod store;

pub use store::{
    forget_membership, record_channels, record_membership, update_roles, update_slot_and_roles,
    NewMembership,
};

#[cfg(test)]
mod tests;
