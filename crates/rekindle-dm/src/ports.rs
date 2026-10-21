//! `GroupDmDeps`: what the group-DM owner needs from its host (plan E2.6).
//! Its impl is the Host adapter in `rekindle-node/src/host/adapters/group_dm/`
//! (slice G2b), over `rekindle_db::repo::group_dms`, the clock and RNG
//! ports, the identity key and the event bus. Nothing here touches Veilid
//! or the transport: the operations return the frames to send and to whom,
//! and the adapter sends them over the pairwise sessions.

use async_trait::async_trait;
use rekindle_codec::capnp_codec::direct::{GroupDmInvite, GroupId};
use rekindle_secrets::group_dm::GroupSenderKey;
use rekindle_types::subscription_events::GroupDmEvent;
use rekindle_utils::RandomSource;

use crate::error::DmError;
use crate::group::Roster;

/// A group we created or accepted (a `group_dms` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupState {
    pub group_id: GroupId,
    pub name: String,
    pub roster: Roster,
    /// Our own sending key's current epoch.
    pub epoch: u32,
    pub created_at_ms: u64,
}

/// An invite we verified and have not answered yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInvite {
    /// The creator, whose session it came over.
    pub from: String,
    pub invite: GroupDmInvite,
}

/// The host's side of the group-DM owner.
#[async_trait]
pub trait GroupDmDeps: Send + Sync {
    /// Our identity key, 64 lowercase hex.
    fn own_identity(&self) -> String;

    /// Milliseconds since the UNIX epoch (the clock port).
    fn now_ms(&self) -> u64;

    /// The engine's generator (the RNG port).
    fn rng(&self) -> &dyn RandomSource;

    /// Our identity key's Ed25519 signature over `message`.
    fn sign_identity(&self, message: &[u8]) -> [u8; 64];

    /// Our nickname for `identity`, or its fingerprint.
    async fn display_name(&self, identity: &str) -> String;

    async fn group(&self, group_id: &GroupId) -> Result<Option<GroupState>, DmError>;

    async fn groups(&self) -> Result<Vec<GroupState>, DmError>;

    async fn insert_group(&self, group: &GroupState) -> Result<(), DmError>;

    /// Store the group's roster and our epoch.
    async fn update_group(
        &self,
        group_id: &GroupId,
        roster: &Roster,
        epoch: u32,
    ) -> Result<(), DmError>;

    /// Delete the group and every key of it.
    async fn delete_group(&self, group_id: &GroupId) -> Result<(), DmError>;

    /// Store `member`'s key of `key.epoch()`.
    async fn put_sender_key(
        &self,
        group_id: &GroupId,
        member: &str,
        key: &GroupSenderKey,
    ) -> Result<(), DmError>;

    /// `member`'s newest key.
    async fn latest_sender_key(
        &self,
        group_id: &GroupId,
        member: &str,
    ) -> Result<Option<GroupSenderKey>, DmError>;

    /// Store `record_key` as `member`'s group chain (its own announcement,
    /// over its pairwise session), replacing the one held.
    async fn put_chain(
        &self,
        group_id: &GroupId,
        member: &str,
        record_key: &str,
    ) -> Result<(), DmError>;

    /// `member`'s group chain, if announced.
    async fn chain(&self, group_id: &GroupId, member: &str) -> Result<Option<String>, DmError>;

    /// Every member's announced group chain, `(member, record_key)`.
    async fn chains(&self, group_id: &GroupId) -> Result<Vec<(String, String)>, DmError>;

    /// Delete every key and the chain of `member` in the group.
    async fn delete_member_keys(&self, group_id: &GroupId, member: &str) -> Result<(), DmError>;

    /// Hold a verified invite until we answer it.
    async fn stash_invite(&self, invite: &PendingInvite) -> Result<(), DmError>;

    /// Every invite we hold and have not answered, unexpired.
    async fn invites(&self) -> Result<Vec<PendingInvite>, DmError>;

    /// Take (and forget) the pending invite to `group_id`.
    async fn take_invite(&self, group_id: &GroupId) -> Result<Option<PendingInvite>, DmError>;

    /// Publish a lifecycle event on the event bus.
    fn publish(&self, event: GroupDmEvent);
}
