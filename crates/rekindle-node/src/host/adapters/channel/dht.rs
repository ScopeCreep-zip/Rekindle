//! Channel record writes and reads over the session's record pool. Every
//! entry is signed with our community pseudonym and written to our slot
//! with the writer keypair derived from the slot seed
//! (`rekindle_protocol::dht::community::channel_record`).

use std::collections::HashMap;
use std::sync::Arc;

use rekindle_channel::deps::{ChannelEntryItem, ChannelWriteContext, DhtWrite};
use rekindle_channel::error::ChannelError;
use rekindle_codec::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_protocol::dht::community::channel_record::{self, AppendOutcome};
use rekindle_protocol::ProtocolError;
use rekindle_records::lease::LeaseId;
use rekindle_records::port::RecordPort;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::id::PseudonymKey;

use super::HostChannel;

/// What a write of our slot needs: the pool and our authorship.
pub(super) struct Writer {
    pub pool: Arc<dyn RecordPort>,
    pub author: PseudonymKey,
    pub signing_key: SigningKey,
}

impl HostChannel {
    pub(super) fn writer(&self, context: &ChannelWriteContext) -> Result<Writer, ChannelError> {
        let credentials = self.pseudonym_credentials_impl(&context.community_id)?;
        Ok(Writer {
            pool: self.records()?,
            author: credentials.pseudonym,
            signing_key: credentials.signing_key,
        })
    }

    /// The community's writer index: the slots of every member the roster
    /// placed, and ours (plan C7.12), ascending.
    fn writer_slots(&self, community_id: &str) -> Vec<u32> {
        let mut slots: Vec<u32> = (self.ctx().communities.members(community_id).into_values())
            .map(|member| member.subkey_index)
            .collect();
        if let Some(ours) =
            (self.gov.community_membership(community_id)).and_then(|m| m.my_subkey_index)
        {
            slots.push(ours);
        }
        slots.sort_unstable();
        slots.dedup();
        slots
    }

    /// Slot subkey → member pseudonym, for every member the roster placed
    /// and us.
    pub(super) fn stage_pseudonyms_impl(&self, community_id: &str) -> HashMap<u32, String> {
        let mut slots: HashMap<u32, String> = (self.ctx().communities.members(community_id))
            .into_iter()
            .map(|(pseudonym, member)| (member.subkey_index, pseudonym))
            .collect();
        if let Some(membership) = self.gov.community_membership(community_id) {
            if let (Some(slot), Some(pseudonym)) =
                (membership.my_subkey_index, membership.my_pseudonym_hex)
            {
                slots.insert(slot, pseudonym);
            }
        }
        slots
    }

    pub(super) async fn create_thread_record_impl(
        &self,
        slot_seed: &[u8; 32],
    ) -> Result<(LeaseId, String), ChannelError> {
        let pool = self.records()?;
        let (lease, record_key, _) = channel_record::create_smpl_channel_record(&*pool, slot_seed)
            .await
            .map_err(|e| ChannelError::Adapter(format!("create thread record: {e}")))?;
        Ok((lease, record_key))
    }

    pub(super) async fn read_entries_impl(
        &self,
        community_id: &str,
        record_key: &str,
    ) -> Result<Vec<ChannelEntryItem>, ChannelError> {
        let pool = self.records()?;
        let slots = self.writer_slots(community_id);
        let items = channel_record::read_all_channel_entries(&*pool, record_key, &slots)
            .await
            .map_err(|e| ChannelError::Adapter(format!("read channel entries: {e}")))?;
        Ok(items
            .into_iter()
            .map(|item| ChannelEntryItem {
                subkey_index: item.subkey_index,
                entry: item.entry,
            })
            .collect())
    }

    pub(super) async fn read_messages_impl(
        &self,
        community_id: &str,
        record_key: &str,
    ) -> Result<Vec<ChannelMessage>, ChannelError> {
        let entries = self.read_entries_impl(community_id, record_key).await?;
        Ok(entries
            .into_iter()
            .filter_map(|item| match item.entry {
                ChannelRecordEntry::Message(message) => Some(message),
                _ => None,
            })
            .collect())
    }
}

/// An append's outcome as the channel crate sees it (plan C7.13): a miss
/// is held by the record pool, and so is a write a lock cut short (the
/// pool persists it), so neither is a failure.
pub fn delivery(
    written: Result<AppendOutcome, ProtocolError>,
    what: &str,
) -> Result<DhtWrite, ChannelError> {
    match written {
        Ok(AppendOutcome::Stored) => Ok(DhtWrite::Stored),
        Ok(AppendOutcome::Held) | Err(ProtocolError::PoolClosed) => Ok(DhtWrite::Held),
        Err(e) => Err(ChannelError::Adapter(format!("{what}: {e}"))),
    }
}
