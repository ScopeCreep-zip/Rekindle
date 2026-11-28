//! The session's writes: each change goes to the store that holds it, then
//! to the projection (plan D1).
//!
//! The governance adapter's trait methods are synchronous, so their
//! database writes are queued in order on the database thread
//! (`Db::fire`; a failure is logged). The vault is written directly, under
//! its lock. Nothing is written while locked. Each change is mirrored into
//! the Host community store, which the governance adapter reads (plan E3.1
//! B2b).

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_db::repo;
use rekindle_db::DbHandle;
use rekindle_governance::state::{ChannelState, GovernanceState};
use rekindle_transport::session::{CommunityMembership, Session};
use rekindle_types::channel::ChannelKind;
use rekindle_vault::{VaultHandle, VaultKey};

use crate::host::community::CommunityStore;

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
            mek_generation,
            parent_voice_channel_id: self.parent_voice_channel_id.as_deref(),
        }
    }
}

/// Store the channels merged governance names: insert-only (a channel
/// whose `ChannelCreated` has not merged yet must not be dropped). A
/// channel has no record (plan E3.3).
pub fn record_channels(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
    state: &GovernanceState,
) {
    let channels: Vec<OwnedChannel> = (state.channels.iter())
        .filter_map(|(id, channel)| OwnedChannel::from_state(hex::encode(id.0), channel))
        .collect();
    if channels.is_empty() {
        return;
    }
    let mek_generation = {
        let guard = session.read();
        let Some(membership) = guard.as_ref().and_then(|s| s.communities.get(community_id)) else {
            return;
        };
        membership.mek_generation
    };
    if let Some((owner, db)) = owner_and_db(session, db) {
        let community = community_id.to_string();
        db.fire("merged channels", move |conn| {
            for channel in &channels {
                repo::channels::insert_if_absent(
                    conn,
                    &owner,
                    &community,
                    &channel.row(mek_generation),
                )?;
            }
            Ok(())
        });
    }
}

/// Adopt the slot index the registry knows for us.
pub fn update_slot(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    communities: &CommunityStore,
    community_id: &str,
    slot_index: u32,
) {
    let changed = {
        let mut guard = session.write();
        let Some(membership) = guard
            .as_mut()
            .and_then(|s| s.communities.get_mut(community_id))
        else {
            return;
        };
        let changed = membership.slot_index != slot_index;
        membership.slot_index = slot_index;
        changed
    };
    if !changed {
        return;
    }
    communities.set_my_slot(community_id, slot_index);
    if let Some((owner, db)) = owner_and_db(session, db) {
        let community = community_id.to_string();
        db.fire("slot", move |conn| {
            repo::communities::set(
                conn,
                &owner,
                &community,
                repo::communities::Column::MySubkeyIndex,
                slot_index,
            )
        });
    }
}

/// Keep the remover's display name with our own removal from the community
/// (`communities.removed_by_name`, A/B R5-2), or clear it (`None`) when the
/// removal turned out void.
pub fn record_removed_by_name(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
    name: Option<String>,
) {
    if let Some((owner, db)) = owner_and_db(session, db) {
        let community = community_id.to_string();
        db.fire("removed by name", move |conn| {
            repo::communities::set(
                conn,
                &owner,
                &community,
                repo::communities::Column::RemovedByName,
                name,
            )
        });
    }
}

/// Commit a consent edit and update the live policy together. The database
/// thread serializes edits, including those submitted by different clients.
///
/// # Errors
/// The community is missing or its policy could not be stored.
pub async fn record_presence_policy(
    db: &rekindle_db::Db,
    owner: String,
    store: Arc<CommunityStore>,
    community: String,
    update: impl FnOnce(&mut rekindle_types::presence::PresenceSharingPolicy) + Send + 'static,
) -> Result<bool, String> {
    db.call(move |conn| {
        let mut map = store.map().write();
        let Some(state) = map.get_mut(&community) else {
            return Ok(None);
        };
        let mut policy = state.presence_policy.clone();
        update(&mut policy);
        if state.presence_policy == policy {
            return Ok(Some(false));
        }
        repo::communities::set_presence_policy(conn, &owner, &community, &policy)?;
        state.presence_policy = policy;
        Ok(Some(true))
    })
    .await
    .map_err(|error| format!("policy not stored: {error}"))?
    .ok_or_else(|| "unknown community".into())
}

/// The next `seq` of our beat in `community_id`, persisted before it is
/// returned (`communities.beat_seq`, plan E3.4 S1). The row is written by
/// [`record_membership`] on the same connection, which runs its calls in
/// order, so a beat after the membership finds it.
///
/// # Errors
/// Locked, no row for the community, or the write failed.
pub async fn next_beat_seq(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
) -> Result<u64, String> {
    let (owner, db) = owner_and_db(session, db).ok_or("no identity is unlocked")?;
    let community = community_id.to_owned();
    db.call(move |conn| repo::communities::next_beat_seq(conn, &owner, &community))
        .await
        .map_err(|e| format!("beat seq: {e}"))
}

/// The display name `pseudonym_hex` has on our stored roster of the
/// community: the name the member list shows (A/B R5-2).
pub async fn roster_name(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    community_id: &str,
    pseudonym_hex: &str,
) -> Option<String> {
    let (owner, db) = owner_and_db(session, db)?;
    let (community, pseudonym) = (community_id.to_owned(), pseudonym_hex.to_owned());
    db.call(move |conn| repo::members::display_name(conn, &owner, &community, &pseudonym))
        .await
        .map_err(|e| tracing::warn!(community = %community_id, error = %e, "roster name"))
        .ok()
        .flatten()
        .filter(|name| !name.is_empty())
}

/// What a new membership stores beside the projection: the registry owner
/// keypair a created community has (none for a joined one) and our member
/// record's secrets.
pub struct NewMembership {
    pub membership: CommunityMembership,
    /// The registry's owner keypair (the vault's `RegistryKeypair`).
    pub registry_keypair: Option<zeroize::Zeroizing<String>>,
    /// Our member record's owner keypair `K_M(p)` (the vault's
    /// `MemberRecordOwner`, plan E3.2).
    pub member_record_owner: Option<zeroize::Zeroizing<String>>,
    /// Our member record's key (`communities.my_member_record`, plan E3.2):
    /// the record `member_record_owner` writes.
    pub member_record: Option<String>,
    /// Our card's X25519 HPKE secret (the vault's `MemberHpkeSecret`).
    pub member_hpke_secret: Option<zeroize::Zeroizing<[u8; 32]>>,
    /// The pinned `GenesisCert` as the anchor holds it
    /// (`communities.genesis_cert`).
    pub genesis_cert: Option<Vec<u8>>,
    /// When it was joined, in ms, by the host's clock.
    pub joined_at: i64,
}

/// Record a membership: its slot seed, registry keypair, member record
/// owner and card HPKE secret into the vault first (a slot seed lost is a slot lost), then its `communities` row,
/// then the projection.
///
/// # Errors
/// Locked, or the vault write failed (nothing is recorded then).
pub fn record_membership(
    session: &RwLock<Option<Session>>,
    db: &DbHandle,
    vault: &VaultHandle,
    communities: &CommunityStore,
    new: NewMembership,
) -> Result<(), String> {
    let (owner, db) = owner_and_db(session, db).ok_or("no identity is unlocked")?;
    let NewMembership {
        membership,
        registry_keypair,
        member_record_owner,
        member_record,
        member_hpke_secret,
        genesis_cert,
        joined_at,
    } = new;
    let community = membership.anchor.clone();
    // Decoded before anything is stored: a cert that does not decode
    // records nothing.
    let cert = genesis_cert
        .as_deref()
        .map(rekindle_codec::governance_op::genesis::decode_genesis)
        .transpose()
        .map_err(|e| format!("genesis cert: {e}"))?;
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
        if let Some(keypair) = &member_record_owner {
            vault
                .put(
                    &VaultKey::MemberRecordOwner {
                        community: community.clone(),
                    },
                    keypair.as_bytes(),
                )
                .map_err(|e| format!("member record owner: {e}"))?;
        }
        if let Some(secret) = &member_hpke_secret {
            rekindle_vault::typed::community_keys::persist_member_hpke_secret(
                vault, &community, secret,
            )
            .map_err(|e| format!("member hpke secret: {e}"))?;
        }
    }
    communities.install_membership(
        &membership,
        registry_keypair.as_ref().map(|key| key.as_str()),
    );
    // The pinned cert every op of ours is built and published under.
    if let Some(decoded) = cert {
        communities.set_genesis_cert(&community, decoded.signed.cert);
    }
    if let Some(record) = &member_record {
        communities.set_my_member_record(&community, record);
    }
    let row = membership.clone();
    db.fire("community membership", move |conn| {
        repo::communities::upsert_membership(
            conn,
            &owner,
            &repo::communities::NewCommunity {
                id: &row.anchor,
                name: &row.community_name,
                joined_at,
                my_pseudonym_key: &row.pseudonym_key,
                mek_generation: i64::try_from(row.mek_generation).unwrap_or(i64::MAX),
                member_registry_key: Some(&row.registry_key),
                my_subkey_index: Some(row.slot_index),
            },
        )?;
        repo::communities::set(
            conn,
            &owner,
            &row.anchor,
            repo::communities::Column::MySegmentIndex,
            row.segment_index.unwrap_or(0),
        )?;
        if let Some(record) = &member_record {
            repo::communities::set(
                conn,
                &owner,
                &row.anchor,
                repo::communities::Column::MyMemberRecord,
                record,
            )?;
        }
        if let Some(cert) = &genesis_cert {
            repo::communities::set(
                conn,
                &owner,
                &row.anchor,
                repo::communities::Column::GenesisCert,
                cert,
            )?;
        }
        Ok(())
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
