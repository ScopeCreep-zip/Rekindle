//! Channel posts read from a channel record rather than notified (A/B
//! finding R2-4, D26): a watched change of a channel record's subkey (the
//! Veilid watch, plan E3.1's third delivery path) and a desktop catch-up
//! read. Every post goes through [`HostChannel::ingest_as`], so the gate's
//! author checks run before anything is opened, stored or published; a
//! removed member's write to its still-writable slot is dropped like its
//! gossip copy.

use rekindle_channel::ChannelMessagingDeps;
use rekindle_codec::community::channel_record::{
    decode_channel_entries, ChannelMessage, ChannelRecordEntry,
};
use rekindle_types::id::ChannelId;

use super::ingest::{Ingested, ReceivedPost};
use super::HostChannel;

impl HostChannel {
    /// The community and channel whose record is
    /// `record_key`, among the communities we hold.
    fn channel_of_record(&self, record_key: &str) -> Option<(String, String)> {
        let communities: Vec<String> = self
            .ctx()
            .communities
            .map()
            .read()
            .keys()
            .cloned()
            .collect();
        communities.into_iter().find_map(|community| {
            let gov = self.governance_state(&community)?;
            let channel = (gov.channels.keys())
                .map(|id| hex::encode(id.0))
                .find(|id| {
                    self.channel_record_key(&community, id).as_deref() == Some(record_key)
                })?;
            Some((community, channel))
        })
    }

    /// A watched change of `record_key`'s `subkeys`: when it is one of our
    /// channel records, every changed slot is read from the network
    /// ([`RecordPort::read_hinted`](rekindle_records::port::RecordPort::read_hinted),
    /// which clears Veilid's report mark, the `ValueChange` contract) and
    /// every post in it is ingested (held posts are duplicates). Returns
    /// whether it was one.
    pub async fn on_record_change(&self, record_key: &str, subkeys: &[u32]) -> bool {
        let Some((community, channel)) = self.channel_of_record(record_key) else {
            return false;
        };
        let Ok(pool) = self.records() else {
            return true;
        };
        // A table hit while the community holds the record for its watch.
        let lease = match pool
            .acquire(record_key, None, rekindle_records::port::OpenIntent::Check)
            .await
        {
            Ok(lease) => lease,
            Err(e) => {
                tracing::debug!(record_key, error = %e, "changed channel record not opened");
                return true;
            }
        };
        let read = pool.read_hinted(lease, subkeys).await;
        pool.release(lease).await;
        let values = match read {
            Ok(values) => values,
            Err(e) => {
                tracing::debug!(record_key, error = %e, "changed channel slots not read");
                return true;
            }
        };
        for (subkey, value) in values {
            let Ok(entries) = decode_channel_entries(&value.data) else {
                continue;
            };
            let posts = entries.into_iter().filter_map(|entry| match entry {
                ChannelRecordEntry::Message(m) => Some((subkey, m)),
                _ => None,
            });
            let seq = u64::from(value.seq);
            self.ingest_read(&community, &channel, record_key, posts, seq, false)
                .await;
        }
        true
    }

    /// Ingest the posts a catch-up read of `record_key` returned (the
    /// desktop's presence catch-up), as history: the gate's backfill rule
    /// keeps a departed author's posts dated before its departure.
    pub async fn ingest_catchup(
        &self,
        community_id: &str,
        channel_id: &str,
        record_key: &str,
        messages: Vec<(u32, ChannelMessage)>,
    ) {
        (self.ingest_read(community_id, channel_id, record_key, messages, 0, true)).await;
    }

    async fn ingest_read(
        &self,
        community_id: &str,
        channel_id: &str,
        record_key: &str,
        posts: impl IntoIterator<Item = (u32, ChannelMessage)>,
        sequence: u64,
        backfill: bool,
    ) {
        if ChannelId::from_hex(channel_id).is_none() {
            return;
        }
        for (subkey_index, message) in posts {
            if message.message_id.is_none() {
                continue;
            }
            let post = ReceivedPost {
                community_id: community_id.to_owned(),
                channel_id: channel_id.to_owned(),
                record_key: record_key.to_owned(),
                subkey_index,
                sequence,
                message,
                forwarded_from_author: None,
            };
            match self.ingest_as(post, backfill).await {
                Ok(Ingested::Dropped(reason)) => {
                    tracing::debug!(community_id, channel_id, ?reason, "record post dropped");
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!(community_id, channel_id, error = %e, "record post not ingested");
                }
            }
        }
    }
}
