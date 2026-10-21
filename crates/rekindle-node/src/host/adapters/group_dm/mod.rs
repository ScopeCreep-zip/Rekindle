//! The group-DM owner's Host side (plan E2.6 G2b): one implementation of
//! `rekindle_dm::GroupDmDeps` over the unlocked identity's database
//! (`repo::group_dms`, `repo::group_dm_invites`), its identity key, the
//! Host's clock and RNG ports and the event journal, used by both hosts
//! through `FriendHost::group_dm`.

pub(crate) mod chain;

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, GroupDmInvite, GroupId};
use rekindle_db::repo::group_dm_invites::{self, InviteRow};
use rekindle_db::repo::group_dms::{self, GroupDmRow};
use rekindle_dm::group::group_id_hex;
use rekindle_dm::{DmError, GroupDmDeps, GroupSenderKey, GroupState, PendingInvite, Roster};
use rekindle_transport::operations::friend::{label, FriendError, FriendHost};
use rekindle_types::subscription_events::{GroupDmEvent, SubscriptionEvent};

use super::friends::HostFriends;

/// The group-DM owner's Host for the identity unlocked when it was opened.
pub struct HostGroupDm {
    host: HostFriends,
    owner: String,
    db: rekindle_db::Db,
    identity: rekindle_crypto::Identity,
}

/// Run a repository call for the owner on the identity's database: `|c, o|`
/// binds the connection and the owner key. A macro so the connection's type
/// is inferred from `Db::call` and the driver is never named
/// (`check-sqlite`).
macro_rules! db {
    ($this:expr, $what:literal, |$c:ident, $o:ident| $body:expr) => {{
        let $o = $this.owner.clone();
        let call = $this.db.call(move |$c| {
            let $o = $o.as_str();
            $body
        });
        async move {
            call.await
                .map_err(|e| DmError::storage(format!("{}: {e}", $what)))
        }
    }};
}

impl HostGroupDm {
    /// The adapter over `host`'s unlocked identity.
    ///
    /// # Errors
    /// `Locked` while no identity is unlocked.
    pub(crate) fn open(host: &HostFriends) -> Result<Self, FriendError> {
        let owner = rekindle_transport::operations::friend::session_owner(host)?;
        let secret = host.identity_secret()?;
        Ok(Self {
            host: host.clone(),
            owner,
            db: host.db()?,
            identity: rekindle_crypto::Identity::from_secret_bytes(&secret),
        })
    }
}

fn parse_group_id(hex_id: &str) -> Result<GroupId, DmError> {
    let bytes = hex::decode(hex_id).map_err(|e| DmError::storage(format!("group id: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| DmError::storage("group id is not 16 bytes"))
}

fn state_of(row: GroupDmRow) -> Result<GroupState, DmError> {
    Ok(GroupState {
        group_id: parse_group_id(&row.group_id)?,
        name: row.name,
        roster: Roster::from_stored(row.roster)?,
        epoch: row.epoch,
        created_at_ms: u64::try_from(row.created_at).unwrap_or_default(),
    })
}

fn row_of(state: &GroupState) -> GroupDmRow {
    GroupDmRow {
        group_id: group_id_hex(&state.group_id),
        name: state.name.clone(),
        roster: state.roster.members().to_vec(),
        epoch: state.epoch,
        created_at: i64::try_from(state.created_at_ms).unwrap_or(i64::MAX),
    }
}

fn invite_of(bytes: &[u8]) -> Result<GroupDmInvite, DmError> {
    match DirectBody::decode(DmTypeId::GroupDmInvite, bytes) {
        Ok(DirectBody::GroupDmInvite(invite)) => Ok(invite),
        Ok(_) => Err(DmError::storage("a held invite is another kind")),
        Err(e) => Err(DmError::storage(format!("held invite: {e}"))),
    }
}

#[async_trait::async_trait]
impl GroupDmDeps for HostGroupDm {
    fn own_identity(&self) -> String {
        self.owner.clone()
    }

    fn now_ms(&self) -> u64 {
        self.host.ctx().clock.now_ms()
    }

    fn rng(&self) -> &dyn rekindle_utils::RandomSource {
        self.host.ctx().rng.as_ref()
    }

    fn sign_identity(&self, message: &[u8]) -> [u8; 64] {
        self.identity.sign(message).to_bytes()
    }

    async fn display_name(&self, identity: &str) -> String {
        label(&self.host, identity)
    }

    async fn group(&self, group_id: &GroupId) -> Result<Option<GroupState>, DmError> {
        let id = group_id_hex(group_id);
        db!(self, "read group", |c, o| group_dms::get(c, o, &id))
            .await?
            .map(state_of)
            .transpose()
    }

    async fn groups(&self) -> Result<Vec<GroupState>, DmError> {
        db!(self, "list groups", |c, o| group_dms::list(c, o))
            .await?
            .into_iter()
            .map(state_of)
            .collect()
    }

    async fn insert_group(&self, group: &GroupState) -> Result<(), DmError> {
        let row = row_of(group);
        db!(self, "store group", |c, o| group_dms::insert(c, o, &row)).await
    }

    async fn update_group(
        &self,
        group_id: &GroupId,
        roster: &Roster,
        epoch: u32,
    ) -> Result<(), DmError> {
        let (id, members) = (group_id_hex(group_id), roster.members().to_vec());
        let held = db!(self, "update group", |c, o| {
            group_dms::update_roster_epoch(c, o, &id, &members, epoch)
        })
        .await?;
        if held {
            Ok(())
        } else {
            Err(DmError::UnknownGroup(group_id_hex(group_id)))
        }
    }

    async fn delete_group(&self, group_id: &GroupId) -> Result<(), DmError> {
        let id = group_id_hex(group_id);
        db!(self, "delete group", |c, o| group_dms::delete(c, o, &id))
            .await
            .map(|_| ())
    }

    async fn put_sender_key(
        &self,
        group_id: &GroupId,
        member: &str,
        key: &GroupSenderKey,
    ) -> Result<(), DmError> {
        let (id, member, epoch, bytes) = (
            group_id_hex(group_id),
            member.to_owned(),
            key.epoch(),
            zeroize::Zeroizing::new(*key.as_bytes()),
        );
        db!(self, "store sender key", |c, o| {
            group_dms::put_sender_key(c, o, &id, &member, epoch, &bytes)
        })
        .await
    }

    async fn latest_sender_key(
        &self,
        group_id: &GroupId,
        member: &str,
    ) -> Result<Option<GroupSenderKey>, DmError> {
        let (id, member) = (group_id_hex(group_id), member.to_owned());
        let held = db!(self, "read sender key", |c, o| {
            group_dms::latest_sender_key(c, o, &id, &member)
        })
        .await?;
        Ok(held.map(|(epoch, key)| GroupSenderKey::new(epoch, key)))
    }

    async fn put_chain(
        &self,
        group_id: &GroupId,
        member: &str,
        record_key: &str,
    ) -> Result<(), DmError> {
        let (id, member, key) = (
            group_id_hex(group_id),
            member.to_owned(),
            record_key.to_owned(),
        );
        db!(self, "store group chain", |c, o| {
            group_dms::put_chain(c, o, &id, &member, &key)
        })
        .await
    }

    async fn chain(&self, group_id: &GroupId, member: &str) -> Result<Option<String>, DmError> {
        Ok(self
            .chains(group_id)
            .await?
            .into_iter()
            .find(|(m, _)| m == member)
            .map(|(_, key)| key))
    }

    async fn chains(&self, group_id: &GroupId) -> Result<Vec<(String, String)>, DmError> {
        let id = group_id_hex(group_id);
        db!(self, "read group chains", |c, o| group_dms::chains(
            c, o, &id
        ))
        .await
    }

    async fn delete_member_keys(&self, group_id: &GroupId, member: &str) -> Result<(), DmError> {
        let (id, member) = (group_id_hex(group_id), member.to_owned());
        db!(self, "delete sender keys", |c, o| {
            group_dms::delete_member_keys(c, o, &id, &member)?;
            group_dms::delete_chain(c, o, &id, &member)
        })
        .await
        .map(|_| ())
    }

    async fn stash_invite(&self, invite: &PendingInvite) -> Result<(), DmError> {
        let row = InviteRow {
            group_id: group_id_hex(&invite.invite.group_id),
            from_key: invite.from.clone(),
            invite: DirectBody::GroupDmInvite(invite.invite.clone()).encode(),
            received_at: self.host.ctx().clock.now_ms_i64(),
        };
        db!(self, "hold invite", |c, o| group_dm_invites::put(
            c, o, &row
        ))
        .await
    }

    async fn take_invite(&self, group_id: &GroupId) -> Result<Option<PendingInvite>, DmError> {
        let (id, now) = (group_id_hex(group_id), self.host.ctx().clock.now_ms_i64());
        let Some(row) = db!(self, "take invite", |c, o| {
            group_dm_invites::take(c, o, &id, now)
        })
        .await?
        else {
            return Ok(None);
        };
        Ok(Some(PendingInvite {
            from: row.from_key,
            invite: invite_of(&row.invite)?,
        }))
    }

    fn publish(&self, event: GroupDmEvent) {
        self.host
            .ctx()
            .publish_event(SubscriptionEvent::GroupDm(event));
    }
}

/// The group-DM adapter, as `FriendHost::group_dm` hands it out.
///
/// # Errors
/// `Locked` while no identity is unlocked.
pub(crate) fn open(host: &HostFriends) -> Result<Arc<dyn GroupDmDeps>, FriendError> {
    Ok(Arc::new(HostGroupDm::open(host)?))
}
