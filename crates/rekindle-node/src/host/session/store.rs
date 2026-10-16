//! The session's writes: each change goes to the store that holds it, then
//! to the projection (plan D1).
//!
//! The governance adapter's trait methods are synchronous, so their
//! database writes are queued in order on the database thread
//! (`Db::fire`; a failure is logged). The vault is written directly, under
//! its lock. Nothing is written while locked.

use parking_lot::RwLock;
use rekindle_db::repo;
use rekindle_db::DbHandle;
use rekindle_governance::state::{ChannelState, GovernanceState};
use rekindle_transport::session::{CommunityMembership, Session};
use rekindle_types::channel::ChannelKind;
use rekindle_vault::{VaultHandle, VaultKey};

/// The unlocked identity's key and database, or `None` while locked.
fn owner_and_db(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
) -> Option<(String, rekindle_db::Db)> {
    let owner = session.read().as_ref()?.identity.public_key_hex.clone();
    Some((owner, db.current().ok()?))
}

/// A merged channel as an owned `channels` row, or `None` (logged) when its
/// kind is not one the table stores.
struct OwnedChannel {
    id: String,
    name: String,
    kind: ChannelKind,
    category_id: Option<String>,
    topic: String,
    slowmode_seconds: u32,
    nsfw: bool,
    record_key: String,
    parent_voice_channel_id: Option<String>,
}

impl OwnedChannel {
    fn from_state(id_hex: String, channel: &ChannelState) -> Option<Self> {
        let Ok(kind) = channel.channel_type.parse::<ChannelKind>() else {
            tracing::warn!(
                channel = %id_hex,
                kind = %channel.channel_type,
                "a governance channel of an unknown kind is not stored"
            );
            return None;
        };
        Some(Self {
            id: id_hex,
            name: channel.name.clone(),
            kind,
            category_id: channel.category_id.map(|c| hex::encode(c.0)),
            topic: channel.topic.clone().unwrap_or_default(),
            slowmode_seconds: channel.slowmode_seconds.unwrap_or(0),
            nsfw: channel.nsfw.unwrap_or(false),
            record_key: channel.record_key.clone(),
            parent_voice_channel_id: channel.parent_voice_channel_id.map(|c| hex::encode(c.0)),
        })
    }

    fn row(&self, mek_generation: u64) -> repo::channels::StoredChannel<'_> {
        repo::channels::StoredChannel {
            id: &self.id,
            name: &self.name,
            channel_type: self.kind.as_str(),
            category_id: self.category_id.as_deref(),
            topic: &self.topic,
            slowmode_seconds: self.slowmode_seconds,
            nsfw: self.nsfw,
            message_record_key: Some(&self.record_key),
            mek_generation,
            parent_voice_channel_id: self.parent_voice_channel_id.as_deref(),
        }
    }
}

/// Record the channels merged governance gives a record key: insert-only,
/// in the projection and the database (a key already known is the same
/// key, and a channel whose `ChannelCreated` has not merged yet must not
/// be dropped). Returns whether the projection changed.
pub fn record_channels(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
    state: &GovernanceState,
) -> bool {
    let channels: Vec<OwnedChannel> = state
        .channels
        .iter()
        .filter(|(_, channel)| !channel.record_key.is_empty())
        .filter_map(|(id, channel)| OwnedChannel::from_state(hex::encode(id.0), channel))
        .collect();
    if channels.is_empty() {
        return false;
    }
    let (changed, mek_generation) = {
        let mut guard = session.write();
        let Some(membership) = guard
            .as_mut()
            .and_then(|s| s.communities.get_mut(community_id))
        else {
            return false;
        };
        let mut changed = false;
        for channel in &channels {
            if !membership.channel_record_keys.contains_key(&channel.id) {
                membership
                    .channel_record_keys
                    .insert(channel.id.clone(), channel.record_key.clone());
                changed = true;
            }
        }
        (changed, membership.mek_generation)
    };
    if let Some((owner, db)) = owner_and_db(session, db) {
        let community = community_id.to_string();
        db.fire("channel record keys", move |conn| {
            for channel in &channels {
                repo::channels::insert_if_absent(
                    conn,
                    &owner,
                    &community,
                    &channel.row(mek_generation),
                )?;
                repo::channels::set_record_key_if_absent(
                    conn,
                    &owner,
                    &community,
                    &channel.id,
                    &channel.record_key,
                )?;
            }
            Ok(())
        });
    }
    changed
}

/// Advance the community's governance clock and return the new value. The
/// stored clock is raised to it in order; a rebuild raises it past every
/// entry of ours the DHT holds, so a lost write cannot hand a published
/// value out again.
///
/// # Errors
/// We are not in the community, or the clock is exhausted.
pub fn next_clock(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
) -> Result<u64, rekindle_types::lamport::LamportError> {
    let next = {
        let mut guard = session.write();
        let membership = guard
            .as_mut()
            .and_then(|s| s.communities.get_mut(community_id))
            .ok_or(rekindle_types::lamport::LamportError::UnknownCommunity)?;
        let next =
            rekindle_types::lamport::LamportClock::new(membership.lamport_counter).increment()?;
        membership.lamport_counter = next;
        next
    };
    store_clock(session, db, community_id, next);
    Ok(next)
}

/// Raise the community's governance clock to `accepted` (never lower).
pub fn raise_clock(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
    accepted: u64,
) {
    let raised = {
        let mut guard = session.write();
        match guard
            .as_mut()
            .and_then(|s| s.communities.get_mut(community_id))
        {
            Some(m) if m.lamport_counter < accepted => {
                m.lamport_counter = accepted;
                true
            }
            _ => false,
        }
    };
    if raised {
        store_clock(session, db, community_id, accepted);
    }
}

fn store_clock(session: &RwLock<Option<Session>>, db: &DbHandle, community_id: &str, clock: u64) {
    let Some((owner, db)) = owner_and_db(session, db) else {
        return;
    };
    let community = community_id.to_string();
    let clock = i64::try_from(clock).unwrap_or(i64::MAX);
    db.fire("governance clock", move |conn| {
        repo::communities::raise_clock(conn, &owner, &community, clock)
    });
}

/// Apply `f` to our role list in the community and store it if it changed.
pub fn update_roles(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
    f: impl FnOnce(&mut Vec<u32>),
) {
    let roles = {
        let mut guard = session.write();
        let Some(membership) = guard
            .as_mut()
            .and_then(|s| s.communities.get_mut(community_id))
        else {
            return;
        };
        let before = membership.role_ids.clone();
        f(&mut membership.role_ids);
        if membership.role_ids == before {
            return;
        }
        membership.role_ids.clone()
    };
    if let Some((owner, db)) = owner_and_db(session, db) {
        let community = community_id.to_string();
        db.fire("my role ids", move |conn| {
            repo::communities::set_my_role_ids(conn, &owner, &community, &roles)
        });
    }
}

/// Adopt the slot index and role ids the registry knows for us.
pub fn update_slot_and_roles(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
    slot_index: u32,
    role_ids: &[u32],
) {
    let changed = {
        let mut guard = session.write();
        let Some(membership) = guard
            .as_mut()
            .and_then(|s| s.communities.get_mut(community_id))
        else {
            return;
        };
        let changed = membership.slot_index != slot_index || membership.role_ids != role_ids;
        membership.slot_index = slot_index;
        membership.role_ids = role_ids.to_vec();
        changed
    };
    if !changed {
        return;
    }
    if let Some((owner, db)) = owner_and_db(session, db) {
        let (community, roles) = (community_id.to_string(), role_ids.to_vec());
        db.fire("slot and roles", move |conn| {
            repo::communities::set(
                conn,
                &owner,
                &community,
                repo::communities::Column::MySubkeyIndex,
                slot_index,
            )?;
            repo::communities::set_my_role_ids(conn, &owner, &community, &roles)
        });
    }
}

/// What a new membership stores beside the projection: the owner keypairs
/// a created community has (none for a joined one).
pub struct NewMembership {
    pub membership: CommunityMembership,
    /// The governance record's owner keypair (`communities.dht_owner_keypair`).
    pub dht_owner_keypair: Option<String>,
    /// The registry's owner keypair (the vault's `RegistryKeypair`).
    pub registry_keypair: Option<String>,
}

/// Record a membership: its slot seed and registry keypair into the vault
/// first (a slot seed lost is a slot lost), then its `communities` row,
/// then the projection.
///
/// # Errors
/// Locked, or the vault write failed (nothing is recorded then).
pub fn record_membership(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    vault: &VaultHandle,
    new: NewMembership,
) -> Result<(), String> {
    let (owner, db) = owner_and_db(session, db).ok_or("no identity is unlocked")?;
    let NewMembership {
        membership,
        dht_owner_keypair,
        registry_keypair,
    } = new;
    let community = membership.governance_key.clone();
    {
        let guard = vault.lock();
        let vault = guard.as_ref().ok_or("no identity is unlocked")?;
        if let Some(seed) = membership.slot_seed {
            vault
                .put(
                    &VaultKey::SlotSeed {
                        community: community.clone(),
                    },
                    hex::encode(seed).as_bytes(),
                )
                .map_err(|e| format!("slot seed: {e}"))?;
        }
        if let Some(keypair) = &registry_keypair {
            vault
                .put(
                    &VaultKey::RegistryKeypair {
                        community: community.clone(),
                    },
                    keypair.as_bytes(),
                )
                .map_err(|e| format!("registry keypair: {e}"))?;
        }
    }
    let row = membership.clone();
    let joined_at = rekindle_utils::timestamp_ms_i64();
    db.fire("community membership", move |conn| {
        repo::communities::insert(
            conn,
            &owner,
            &repo::communities::NewCommunity {
                id: &row.governance_key,
                name: &row.community_name,
                my_role_ids: &row.role_ids,
                joined_at,
                dht_owner_keypair: dht_owner_keypair.as_deref(),
                my_pseudonym_key: &row.pseudonym_key,
                mek_generation: i64::try_from(row.mek_generation).unwrap_or(i64::MAX),
                member_registry_key: Some(&row.registry_key),
                my_subkey_index: Some(row.slot_index),
                governance_key: Some(&row.governance_key),
            },
        )?;
        repo::communities::set(
            conn,
            &owner,
            &row.governance_key,
            repo::communities::Column::MySegmentIndex,
            row.segment_index.unwrap_or(0),
        )?;
        repo::communities::raise_clock(
            conn,
            &owner,
            &row.governance_key,
            i64::try_from(row.lamport_counter).unwrap_or(i64::MAX),
        )
    });
    if let Some(s) = session.write().as_mut() {
        s.join_community(membership);
    }
    Ok(())
}

/// Forget a membership we left: its row (its channels go with it), its
/// slot seed and registry keypair, and the projection's entry.
///
/// # Errors
/// Locked, or a store could not be written.
pub async fn forget_membership(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    vault: &VaultHandle,
    community_id: &str,
) -> Result<(), String> {
    let (owner, db) = owner_and_db(session, db).ok_or("no identity is unlocked")?;
    let community = community_id.to_string();
    db.call(move |conn| repo::communities::delete(conn, &owner, &community))
        .await
        .map_err(|e| format!("community row: {e}"))?;
    {
        let guard = vault.lock();
        if let Some(vault) = guard.as_ref() {
            for key in [
                VaultKey::SlotSeed {
                    community: community_id.to_string(),
                },
                VaultKey::RegistryKeypair {
                    community: community_id.to_string(),
                },
            ] {
                vault
                    .delete(&key)
                    .map_err(|e| format!("community keys: {e}"))?;
            }
        }
    }
    if let Some(s) = session.write().as_mut() {
        s.leave_community(community_id);
    }
    Ok(())
}
