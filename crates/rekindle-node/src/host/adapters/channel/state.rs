//! Community-store reads and mutations for the channel pipeline. Every
//! function clones what it needs and drops the guard before returning:
//! `parking_lot` guards are `!Send`.

use std::collections::HashMap;

use rekindle_channel::deps::{
    ChannelInfoSnapshot, ChannelWriteContext, MemberMentionView, PseudonymCredentials,
    RoleSnapshot, ThreadStateSnapshot,
};
use rekindle_channel::error::ChannelError;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_types::id::{ChannelId, PseudonymKey, ThreadId};
use rekindle_types::lamport::{LamportClock, LamportError};

use super::HostChannel;
use crate::host::community::state::ChannelType;

impl HostChannel {
    pub(super) fn my_role_ids_impl(&self, community_id: &str) -> Vec<u32> {
        let map = self.ctx().communities.map().read();
        map.get(community_id)
            .map(|c| c.my_role_ids.clone())
            .unwrap_or_default()
    }

    pub(super) fn my_pseudonym_impl(&self, community_id: &str) -> Option<String> {
        self.gov
            .community_membership(community_id)?
            .my_pseudonym_hex
    }

    /// Our community pseudonym and its signing key, derived from the
    /// identity secret and the community (the authorship of every entry).
    pub(super) fn pseudonym_credentials_impl(
        &self,
        community_id: &str,
    ) -> Result<PseudonymCredentials, ChannelError> {
        let secret = (self.ctx().identity_secret())
            .ok_or_else(|| ChannelError::Adapter("no identity unlocked".into()))?;
        let signing_key =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
        Ok(PseudonymCredentials {
            pseudonym: PseudonymKey(signing_key.verifying_key().to_bytes()),
            signing_key,
        })
    }

    pub(super) fn channel_info_impl(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Option<ChannelInfoSnapshot> {
        let map = self.ctx().communities.map().read();
        let community = map.get(community_id)?;
        let channel = community.channels.iter().find(|c| c.id == channel_id)?;
        Some(ChannelInfoSnapshot {
            channel_id: channel.id.clone(),
            channel_type: channel.channel_type.to_string(),
            slowmode_seconds: channel.slowmode_seconds,
            last_send_at_ms: community.channel_last_send_at.get(channel_id).copied(),
            is_forum: matches!(channel.channel_type, ChannelType::Forum),
        })
    }

    /// Where our write of `channel_id` lands: the channel record of our
    /// segment (segment 0's from `ChannelCreated`, a later one's from
    /// `ChannelSegmentLinked`), our slot and its derived writer keypair.
    pub(super) fn channel_write_context_impl(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<ChannelWriteContext, ChannelError> {
        let membership = (self.gov.community_membership(community_id))
            .ok_or_else(|| ChannelError::CommunityNotFound(community_id.into()))?;
        let segment_index = membership.my_segment_index.unwrap_or(0);
        let channel_key = if segment_index == 0 {
            membership.channel_log_keys.get(channel_id).cloned()
        } else {
            let channel = ChannelId::from_hex(channel_id)
                .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
            self.gov.governance_state(community_id).and_then(|gov| {
                (gov.channel_segment_records.get(&(channel, segment_index)))
                    .map(|record| record.record_key.clone())
            })
        }
        .ok_or_else(|| {
            ChannelError::Adapter(format!(
                "no channel record for segment {segment_index} of channel {channel_id}"
            ))
        })?;
        Ok(ChannelWriteContext {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            channel_key,
            slot_keypair_str: (membership.slot_keypair)
                .ok_or_else(|| ChannelError::Adapter("slot keypair missing".into()))?,
            slot_index: (membership.my_subkey_index)
                .ok_or_else(|| ChannelError::Adapter("community subkey index missing".into()))?,
            segment_index,
        })
    }

    pub(super) fn channel_record_key_impl(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Option<String> {
        (self
            .gov
            .community_membership(community_id)?
            .channel_log_keys)
            .get(channel_id)
            .cloned()
    }

    pub(super) fn thread_state_impl(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Option<ThreadStateSnapshot> {
        let gov = self.gov.governance_state(community_id)?;
        let id: [u8; 16] = hex::decode(thread_id).ok()?.try_into().ok()?;
        let thread = gov.threads.get(&ThreadId(id))?;
        Some(ThreadStateSnapshot {
            thread_id: thread_id.to_string(),
            parent_channel_id_hex: hex::encode(thread.parent_channel_id.0),
            name: thread.name.clone(),
            thread_type: thread.thread_type.clone(),
            record_key: thread.record_key.clone(),
            invited: thread.invited.clone(),
            forum_tag: thread.forum_tag.clone(),
            auto_archive_seconds: thread.auto_archive_seconds,
            created_lamport: thread.created.lamport,
            archived_lamport: thread.archived.map(|stamp| stamp.lamport),
            creator_pseudonym_hex: hex::encode(thread.creator.0),
        })
    }

    pub(super) fn slot_seed_impl(&self, community_id: &str) -> Option<[u8; 32]> {
        let seed = self.gov.community_membership(community_id)?.slot_seed_hex?;
        hex::decode(seed).ok()?.try_into().ok()
    }

    pub(super) fn member_profile_impl(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
    ) -> MemberMentionView {
        let map = self.ctx().communities.map().read();
        let Some(community) = map.get(community_id) else {
            return MemberMentionView::default();
        };
        MemberMentionView {
            display_name: (community.member_profiles.get(pseudonym_hex))
                .and_then(|p| p.display_name.clone()),
            role_ids: (community.member_roles.get(pseudonym_hex))
                .cloned()
                .unwrap_or_default(),
        }
    }

    pub(super) fn list_member_profiles_impl(
        &self,
        community_id: &str,
    ) -> HashMap<String, MemberMentionView> {
        let map = self.ctx().communities.map().read();
        let Some(community) = map.get(community_id) else {
            return HashMap::new();
        };
        (community.member_profiles.iter())
            .map(|(pseudonym, profile)| {
                let view = MemberMentionView {
                    display_name: profile.display_name.clone(),
                    role_ids: (community.member_roles.get(pseudonym))
                        .cloned()
                        .unwrap_or_default(),
                };
                (pseudonym.clone(), view)
            })
            .collect()
    }

    pub(super) fn community_roles_impl(&self, community_id: &str) -> Vec<RoleSnapshot> {
        let map = self.ctx().communities.map().read();
        map.get(community_id)
            .map(|c| {
                (c.roles.iter())
                    .map(|r| RoleSnapshot {
                        id: r.id,
                        name: r.name.clone(),
                        mentionable: r.mentionable,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Our effective permissions from merged governance, at the Host's clock.
    pub(super) fn compute_my_permissions_impl(&self, community_id: &str) -> u64 {
        let Some(gov) = self.gov.governance_state(community_id) else {
            return 0;
        };
        let Some(pseudonym) = (self.my_pseudonym_impl(community_id))
            .and_then(|hex| hex::decode(hex).ok())
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        else {
            return 0;
        };
        rekindle_governance::permissions::compute_permissions(
            &PseudonymKey(pseudonym),
            None,
            &gov,
            self.ctx().clock.now_secs(),
        )
    }

    /// Tick a per-channel counter (`key` a channel id, or the thread key).
    pub(super) fn next_sequence_impl(&self, community_id: &str, key: &str) -> u64 {
        let mut map = self.ctx().communities.map().write();
        let Some(community) = map.get_mut(community_id) else {
            return 1;
        };
        let sequence = community
            .channel_sequences
            .entry(key.to_string())
            .or_insert(0);
        *sequence += 1;
        *sequence
    }

    pub(super) fn mark_last_send_at_impl(&self, community_id: &str, channel_id: &str, now: i64) {
        if let Some(community) = self.ctx().communities.map().write().get_mut(community_id) {
            community
                .channel_last_send_at
                .insert(channel_id.to_string(), now);
        }
    }

    /// Tick the community's message clock, the clock inbound gossip merges
    /// into, so a send orders after everything we have received. The GUI's
    /// gossip merges into the store's `message_clock`; `rekindled`'s into
    /// its subscription mesh. One clock when the gossip mesh moves to the
    /// Host (E3.4).
    pub(super) fn increment_lamport_impl(&self, community_id: &str) -> Result<u64, LamportError> {
        if self.ctx().shell.is_some() {
            let mut map = self.ctx().communities.map().write();
            let community = (map.get_mut(community_id)).ok_or(LamportError::UnknownCommunity)?;
            let next = LamportClock::new(community.message_clock).increment()?;
            community.message_clock = next;
            return Ok(next);
        }
        let subscriptions = self.ctx().subscriptions.read();
        (subscriptions.as_ref())
            .ok_or(LamportError::UnknownCommunity)?
            .next_message_lamport(community_id)
    }
}
