//! The daemon's [`Session`], projected from the unlocked identity's stores
//! (plan D1: `session.json` is retired).
//!
//! [`load`] builds it at unlock from the identity's database and vault:
//! the identity row and its friend inbox, one membership per `communities`
//! row (its channels' record keys from `channels`, its slot seed from the
//! vault), the friends' DM log keys and the pending friend requests. Every
//! change after that is written to the store that holds it, then applied
//! to the projection; nothing writes the projection back as a whole.

use std::collections::HashMap;

use rekindle_db::repo;
use rekindle_db::Db;
use rekindle_transport::session::{
    CommunityMembership, PendingFriendRequest, Session, SessionIdentity,
};
use rekindle_vault::typed::community_keys::load_slot_seed;
use rekindle_vault::typed::owner_keys::{load_owner_keypair, OwnerKey};
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
    inbox: Option<(String, String)>,
    communities: Vec<(
        repo::communities::CommunityRow,
        Vec<u32>,
        Vec<(String, String)>,
    )>,
    dm_log_keys: Vec<(String, String)>,
    pending: Vec<(
        repo::pending_requests::PendingFriendRequest,
        repo::pending_requests::Answer,
    )>,
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
            let inbox = repo::identity::friend_inbox(conn, &o)?;
            let mut communities = Vec::new();
            for row in repo::communities::load_all(conn, &o)? {
                let role_ids =
                    repo::communities::my_role_ids(conn, &o, &row.id)?.unwrap_or_default();
                let channels = repo::channels::record_keys(conn, &o, &row.id)?;
                communities.push((row, role_ids, channels));
            }
            let dm_log_keys = repo::friends::dm_log_keys(conn, &o)?;
            let mut pending = Vec::new();
            for request in repo::pending_requests::list(conn, &o)? {
                let answer = repo::pending_requests::answer(conn, &o, &request.public_key)?;
                pending.push((request, answer));
            }
            Ok(Some(Rows {
                display_name,
                records,
                inbox,
                communities,
                dm_log_keys,
                pending,
            }))
        })
        .await
        .map_err(|e| SessionError::Db(e.to_string()))?
        .ok_or(SessionError::NoIdentityRow)?;

    let guard = vault.lock();
    let vault = guard
        .as_ref()
        .ok_or_else(|| SessionError::Vault("locked".into()))?;
    let keypair =
        |record| load_owner_keypair(vault, record).map_err(|e| SessionError::Vault(e.to_string()));
    let (friend_inbox_key, friend_inbox_keypair_hex) = rows.inbox.unwrap_or_default();
    let identity = SessionIdentity {
        public_key_hex: owner.to_string(),
        display_name: rows.display_name,
        profile_dht_key: rows.records.profile.unwrap_or_default(),
        mailbox_dht_key: rows.records.mailbox.unwrap_or_default(),
        friend_list_dht_key: rows.records.friend_list.unwrap_or_default(),
        friend_inbox_key,
        friend_inbox_keypair_hex,
        profile_owner_keypair: keypair(OwnerKey::Profile)?,
        friend_list_owner_keypair: keypair(OwnerKey::FriendList)?,
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
    session.dm_log_keys = rows.dm_log_keys.into_iter().collect();
    session.pending_friend_requests = rows
        .pending
        .into_iter()
        .map(|(request, answer)| PendingFriendRequest {
            public_key: request.public_key,
            display_name: request.display_name,
            message: request.message,
            profile_dht_key: answer.profile_dht_key.unwrap_or_default(),
            route_blob: answer.route_blob.unwrap_or_default(),
            mailbox_dht_key: answer.mailbox_dht_key.unwrap_or_default(),
            prekey_bundle: answer.prekey_bundle.unwrap_or_default(),
            invite_id: answer.invite_id,
            received_at: u64::try_from(request.received_at).unwrap_or(0),
        })
        .collect();
    Ok(session)
}

/// Record an inbound friend request: in the projection at once (the
/// subscription manager reads it next), and in the identity's database,
/// queued in order on its thread (a failed write is logged). Nothing is
/// recorded while locked.
pub fn record_pending_request(
    session: &parking_lot::RwLock<Option<Session>>,
    db: &rekindle_db::DbHandle,
    request: PendingFriendRequest,
) {
    let owner = {
        let mut guard = session.write();
        let Some(session) = guard.as_mut() else {
            return;
        };
        session.add_pending_friend_request(request.clone());
        session.identity.public_key_hex.clone()
    };
    let Ok(db) = db.current() else {
        return;
    };
    db.fire("pending friend request", move |conn| {
        repo::pending_requests::upsert(
            conn,
            &owner,
            &repo::pending_requests::Incoming {
                public_key: &request.public_key,
                display_name: &request.display_name,
                message: &request.message,
                received_at: i64::try_from(request.received_at).unwrap_or(i64::MAX),
                profile_dht_key: &request.profile_dht_key,
                route_blob: &request.route_blob,
                mailbox_dht_key: &request.mailbox_dht_key,
                prekey_bundle: &request.prekey_bundle,
                invite_id: request.invite_id.as_deref(),
            },
        )
    });
}

mod store;

pub use store::{
    forget_membership, next_clock, raise_clock, record_channels, record_membership, update_roles,
    update_slot_and_roles, NewMembership,
};

#[cfg(test)]
mod tests;
