//! The daemon's [`Session`], projected from the unlocked identity's stores
//! (plan D1: the daemon's session file is retired).
//!
//! [`load`] builds it at unlock from the identity's database and vault:
//! the identity row, one membership per `communities` row (its channels'
//! record keys from `channels`, its slot seed from the vault), and the
//! friends (plan E2.1; pending requests are read where they are answered,
//! `repo::pending_requests`). Every
//! change after that is written to the store that holds it, then applied
//! to the projection; nothing writes the projection back as a whole.

use std::collections::HashMap;

use rekindle_db::repo;
use rekindle_db::Db;
use rekindle_transport::session::{CommunityMembership, Session, SessionIdentity};
use rekindle_types::friend::FriendState;
use rekindle_vault::typed::community_keys::load_slot_seed;
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
    communities: Vec<(
        repo::communities::CommunityRow,
        Vec<u32>,
        Vec<(String, String)>,
    )>,
    friends: Vec<repo::friends::FriendRow>,
}

/// A slot seed as stored (hex), or `None` when it does not decode.
fn slot_seed(hex_seed: &str) -> Option<[u8; 32]> {
    hex::decode(hex_seed).ok()?.try_into().ok()
}

/// Build the session of the unlocked identity `owner`.
///
/// # Errors
/// The identity row is missing, or a store cannot be read.
pub async fn load(db: &Db, vault: &VaultHandle, owner: &str) -> Result<Session, SessionError> {
    let o = owner.to_string();
    let rows = db
        .call(move |conn| {
            let Some((display_name, records)) = repo::identity::login_row(conn, &o)? else {
                return Ok(None);
            };
            let mut communities = Vec::new();
            for row in repo::communities::load_all(conn, &o)? {
                let role_ids =
                    repo::communities::my_role_ids(conn, &o, &row.id)?.unwrap_or_default();
                let channels = repo::channels::record_keys(conn, &o, &row.id)?;
                communities.push((row, role_ids, channels));
            }
            let friends = repo::friends::load_all(conn, &o)?;
            Ok(Some(Rows {
                display_name,
                records,
                communities,
                friends,
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
        profile_dht_key: rows.records.profile.unwrap_or_default(),
        mailbox_dht_key: rows.records.mailbox.unwrap_or_default(),
        friend_list_dht_key: rows.records.friend_list.unwrap_or_default(),
    };

    let mut session = Session::new(identity);
    for (row, role_ids, channels) in rows.communities {
        let seed = load_slot_seed(vault, &row.id).and_then(|hex_seed| slot_seed(&hex_seed));
        session.join_community(CommunityMembership {
            governance_key: row.id,
            pseudonym_key: row.my_pseudonym_key.unwrap_or_default(),
            role_ids,
            registry_key: row.member_registry_key.unwrap_or_default(),
            slot_index: row.my_subkey_index.unwrap_or(0),
            community_name: row.name,
            slot_seed: seed,
            channel_record_keys: channels.into_iter().collect::<HashMap<_, _>>(),
            segment_index: row.my_segment_index,
            lamport_counter: row.governance_clock,
            mek_generation: row.mek_generation,
        });
    }
    drop(guard);
    session.friends = rows
        .friends
        .into_iter()
        .map(|row| (row.public_key.clone(), friend_state(row)))
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
    forget_membership, next_clock, raise_clock, record_channels, record_membership, update_roles,
    update_slot_and_roles, NewMembership,
};

#[cfg(test)]
mod tests;
