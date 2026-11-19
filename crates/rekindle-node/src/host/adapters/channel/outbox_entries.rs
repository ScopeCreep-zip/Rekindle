//! `read_author_outboxes(channel)` (plan E3.3 "Files move onto MSR"): the
//! entries of one channel across every author's channel outbox (`M`/4),
//! for the files paths that look an entry up rather than apply it (an
//! attachment's offer, a holder's `AttachmentCached`). Each entry is
//! verified under its author at its chain seq and linked to its channel by
//! tag, as the reader does ([`super::outbox_read`]); a message naming
//! another author is dropped. A departed author's entries count only up to
//! the highest head its departures recorded of the outbox (what the reader
//! keeps, `outbox_read.rs` "Standing").

use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_codec::member_record::{decode_outbox_entry, ChainPosition};
use rekindle_records::chain::read_log_after;
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::id::PseudonymKey;

use super::HostChannel;

impl HostChannel {
    /// Every entry of `channel_id` in `community_id`'s channel outboxes,
    /// author by author in chain order. An outbox we cannot read now is
    /// skipped (the lookup is retried by its caller).
    pub(crate) async fn read_author_outboxes(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Vec<ChannelRecordEntry> {
        let (Some(state), Ok(port)) = (self.governance_state(community_id), self.records()) else {
            return Vec::new();
        };
        let members = (state.members.iter())
            .map(|(author, member)| (author.clone(), member.member_record.clone(), u64::MAX));
        let departed = (state.departed_heads.iter()).filter_map(|(author, heads)| {
            let record = state.op_log_records.get(author)?;
            let (head, _) = heads.get(record)?.iter().next_back()?;
            Some((author.clone(), record.clone(), u64::from(*head)))
        });
        // In author order, as the catch-up reads (`catch_up_outboxes`).
        let mut authors: Vec<_> = members.chain(departed).collect();
        authors.sort_unstable_by_key(|(author, _, _)| author.0);
        let mut entries = Vec::new();
        for (author, record, through) in authors {
            match read_log_after(&*port, &record, OUTBOX, 0).await {
                Ok(read) => entries.extend(
                    (read.entries.iter())
                        .filter(|e| e.seq <= through)
                        .filter_map(|e| {
                            self.entry_of_channel(community_id, channel_id, &author, e)
                        }),
                ),
                Err(error) => {
                    tracing::debug!(community_id, record, %error, "channel outbox not read for a lookup");
                }
            }
        }
        entries
    }

    /// `entry` of `author`'s chain, when it verifies and is of `channel_id`.
    fn entry_of_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        author: &PseudonymKey,
        entry: &rekindle_records::chain::ChainEntry,
    ) -> Option<ChannelRecordEntry> {
        let at = ChainPosition {
            anchor: community_id,
            chain_seq: entry.seq,
        };
        let decoded = decode_outbox_entry(&entry.ciphertext, at, &author.0).ok()?;
        let channel =
            self.channel_of_tag(community_id, decoded.generation, &decoded.channel_tag)?;
        if channel != channel_id {
            return None;
        }
        match &decoded.entry {
            ChannelRecordEntry::Message(m) if m.sender_pseudonym != hex::encode(author.0) => None,
            _ => Some(decoded.entry),
        }
    }
}
