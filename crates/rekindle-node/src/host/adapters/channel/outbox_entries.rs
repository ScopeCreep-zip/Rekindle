//! `read_author_outboxes(channel)` (plan E3.3 "Files move onto MSR"): the
//! entries of one channel (or thread) across every author's channel outbox (`M`/4),
//! for the files paths that look an entry up rather than apply it (an
//! attachment's offer, a holder's `AttachmentCached`). Each entry is
//! verified under its author at its chain seq and linked to its channel by
//! tag, as the reader does ([`super::outbox_read`]); a message naming
//! another author is dropped. A departed author's entries count only up to
//! the highest head its departures recorded of the outbox (what the reader
//! keeps, `outbox_read.rs` "Standing").

use rekindle_channel::deps::OutboxItem;
use rekindle_channel::outbox_seal::stream_of_tag;
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_codec::member_record::{decode_outbox_entry, ChainPosition};
use rekindle_records::chain::read_log_after;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::id::{ChannelId, PseudonymKey, ThreadId};

use super::{admission::admission_view, HostChannel};

impl HostChannel {
    /// Every entry of `channel_id` in `community_id`'s channel outboxes,
    /// author by author in chain order. An outbox we cannot read now is
    /// skipped (the lookup is retried by its caller).
    pub(crate) async fn read_author_outboxes(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Vec<OutboxItem> {
        let (Some(state), Ok(port)) = (self.governance_state(community_id), self.records()) else {
            return Vec::new();
        };
        let members = (state.members.iter())
            .map(|(author, member)| (author.clone(), member.member_record.clone(), false));
        let departed = (state.departed_heads.iter()).filter_map(|(author, heads)| {
            let record = state.op_log_records.get(author)?;
            heads.get(record)?;
            Some((author.clone(), record.clone(), true))
        });
        // In author order, as the catch-up reads (`catch_up_outboxes`).
        let mut authors: Vec<_> = members.chain(departed).collect();
        authors.sort_unstable_by_key(|(author, _, _)| author.0);
        let thread = ChannelId::from_hex(channel_id)
            .is_some_and(|id| state.threads.contains_key(&ThreadId(id.0)));
        let blocked = match self.blocked_members(community_id).await {
            Ok(blocked) => blocked,
            Err(error) => {
                tracing::warn!(community_id, %error, "post author blocks not read");
                return Vec::new();
            }
        };
        let mut entries = Vec::new();
        for (author, record, departed) in authors {
            if state.bans.contains(&author) {
                continue;
            }
            let through = if departed {
                let Some(through) = self
                    .departed_lookup_through(&*port, community_id, &author, &record)
                    .await
                else {
                    continue;
                };
                through
            } else {
                u64::MAX
            };
            let author_blocked = blocked.contains(&hex::encode(author.0));
            match read_log_after(&*port, &record, OUTBOX, 0).await {
                Ok(read) => {
                    for entry in read.entries.iter().filter(|entry| entry.seq <= through) {
                        if let Some(item) = self
                            .entry_of_channel(
                                community_id,
                                channel_id,
                                &author,
                                entry,
                                thread,
                                author_blocked,
                            )
                            .await
                        {
                            entries.push(item);
                        }
                    }
                }
                Err(error) => {
                    tracing::debug!(community_id, record, %error, "channel outbox not read for a lookup");
                }
            }
        }
        entries
    }

    /// `entry` of `author`'s chain, when it verifies and its tag is that of
    /// the stream `channel_id` (a channel or a thread).
    async fn entry_of_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        author: &PseudonymKey,
        entry: &rekindle_records::chain::ChainEntry,
        thread: bool,
        blocked: bool,
    ) -> Option<OutboxItem> {
        let at = ChainPosition {
            anchor: community_id,
            chain_seq: entry.seq,
        };
        let decoded = decode_outbox_entry(&entry.ciphertext, at, &author.0).ok()?;
        let scope = self.stream_scope(community_id, channel_id)?;
        let keys = self.keys();
        stream_of_tag(
            &*keys,
            community_id,
            [(channel_id, scope)],
            decoded.generation,
            &decoded.channel_tag,
        )?;
        let post = match &decoded.entry {
            ChannelRecordEntry::Message(m) => Some((&m.sender_pseudonym, m.timestamp)),
            ChannelRecordEntry::Forward(f) => Some((&f.sender_pseudonym, f.timestamp)),
            _ => None,
        };
        let mut admission = None;
        if let Some((sender, timestamp)) = post {
            if blocked || sender != &hex::encode(author.0) {
                return None;
            }
            admission = self
                .post_admission(
                    community_id,
                    channel_id,
                    author,
                    &decoded.governance_cut,
                    thread,
                    timestamp,
                )
                .await
                .inspect_err(|error| {
                    tracing::warn!(community_id, channel_id, %error, "post cut not read");
                })
                .ok()?;
            if admission.is_some() && !thread {
                return None;
            }
        }
        Some(OutboxItem {
            author: author.clone(),
            stream_seq: decoded.stream_seq,
            committed_at: decoded.committed_at,
            entry: decoded.entry,
            admission: admission.map(admission_view),
        })
    }

    pub(super) fn post_stream_of_tag(
        &self,
        community: &str,
        generation: u64,
        tag: &[u8; 16],
    ) -> Option<(String, Option<String>)> {
        if let Some(channel) = self.channel_of_tag(community, generation, tag) {
            return Some((channel, None));
        }
        let state = self.governance_state(community)?;
        let keys = self.keys();
        let threads: Vec<_> = state
            .threads
            .iter()
            .map(|(id, thread)| (hex::encode(id.0), thread.parent_channel_id))
            .collect();
        let streams = threads
            .iter()
            .map(|(id, parent)| (id.as_str(), keys.scope_for_text(community, *parent)));
        let matched = stream_of_tag(&*keys, community, streams, generation, tag)?;
        let (_, parent) = threads.iter().find(|(id, _)| id == matched)?;
        Some((matched.to_owned(), Some(parent.to_hex())))
    }

    /// The text scope of the stream `stream_id`: a channel's own, or a
    /// thread's parent channel's (plan E3.3 "Channel tag").
    fn stream_scope(&self, community_id: &str, stream_id: &str) -> Option<KeyScope> {
        let state = self.governance_state(community_id)?;
        let id: [u8; 16] = hex::decode(stream_id).ok()?.try_into().ok()?;
        let channel = if state.channels.contains_key(&ChannelId(id)) {
            ChannelId(id)
        } else {
            state.threads.get(&ThreadId(id))?.parent_channel_id
        };
        Some(self.keys().scope_for_text(community_id, channel))
    }
}
