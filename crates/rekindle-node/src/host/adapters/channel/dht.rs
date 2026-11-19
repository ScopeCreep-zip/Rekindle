//! Channel record reads over the session's record pool (older slot
//! history; nothing writes the channel record since E3.3 slice 8c).

use rekindle_channel::deps::ChannelEntryItem;
use rekindle_channel::error::ChannelError;
use rekindle_codec::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_protocol::dht::community::channel_record;

use super::HostChannel;

impl HostChannel {
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
                seq: item.seq,
                slot_digest: item.slot_digest,
                author: item.author,
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
