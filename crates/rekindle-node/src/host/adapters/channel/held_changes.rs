//! The channel changes that arrive before their message (plan E3.3 E5),
//! and the moderator deletes a landing post meets (M4).
//!
//! An author's edit or delete whose message we do not hold waits in the
//! community store's bounded [`HeldChanges`](rekindle_channel::held::HeldChanges)
//! cache, keyed by (author, message id); when the message lands it is
//! re-validated by the same rules (E1–E3) and the same gate as a change
//! read with its message.

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_types::id::PseudonymKey;

use super::changes::ChangeSource;
use super::ingest::ReceivedPost;
use super::HostChannel;

/// One change held for its message: where it was read, and the entry.
#[derive(Debug, Clone)]
pub(crate) struct HeldChange {
    community_id: String,
    channel_id: String,
    stream_seq: u64,
    author: PseudonymKey,
    entry: ChannelRecordEntry,
}

/// What a held change costs the cache: its sealed body and its ids.
fn held_size(change: &HeldChange) -> usize {
    let body = match &change.entry {
        ChannelRecordEntry::Edit(edit) => edit.ciphertext.len() + edit.message_id.len(),
        ChannelRecordEntry::Delete(delete) => delete.message_id.len(),
        _ => 0,
    };
    body + change.community_id.len() + change.channel_id.len() + 64
}

fn message_of(entry: &ChannelRecordEntry) -> Option<&str> {
    match entry {
        ChannelRecordEntry::Edit(edit) => Some(&edit.message_id),
        ChannelRecordEntry::Delete(delete) => Some(&delete.message_id),
        _ => None,
    }
}

impl HostChannel {
    /// Hold `entry`, read at `source`, until its message lands (E5).
    pub(super) fn hold_change(&self, source: ChangeSource<'_>, entry: &ChannelRecordEntry) {
        let Some(message_id) = message_of(entry) else {
            return;
        };
        let change = HeldChange {
            community_id: source.community_id.to_owned(),
            channel_id: source.channel_id.to_owned(),
            stream_seq: source.stream_seq,
            author: source.author.clone(),
            entry: entry.clone(),
        };
        let size = held_size(&change);
        let author = hex::encode(source.author.0);
        (self.ctx().communities.held_changes().lock()).hold(&author, message_id, change, size);
    }

    /// The message `message_id` by `author` landed in `community_id`: apply
    /// every change of it that waited, deletes first, each re-validated.
    pub(super) async fn release_held(
        &self,
        community_id: &str,
        author: &PseudonymKey,
        message_id: &str,
    ) {
        let held =
            (self.ctx().communities.held_changes().lock()).take(&hex::encode(author.0), message_id);
        let mut held: Vec<HeldChange> = (held.into_iter())
            .filter(|change| change.community_id == community_id)
            .collect();
        held.sort_by_key(|c| {
            (
                !matches!(c.entry, ChannelRecordEntry::Delete(_)),
                c.entry.lamport(),
            )
        });
        for change in held {
            let source = ChangeSource {
                community_id: &change.community_id,
                channel_id: &change.channel_id,
                stream_seq: change.stream_seq,
                author: &change.author,
            };
            self.apply_changes(source, std::slice::from_ref(&change.entry))
                .await;
        }
    }

    /// The moderator whose merged `AdminDelete` removed `post`, when one
    /// did (plan E3.3 M4).
    pub(super) fn removed_by_moderator(
        &self,
        post: &ReceivedPost,
        message_id: &str,
    ) -> Option<PseudonymKey> {
        let state = self.gov.governance_state(&post.community_id)?;
        crate::host::adapters::governance::channel_messages::removed_by(
            &state,
            &post.channel_id,
            message_id,
            &post.message.sender_pseudonym,
        )
    }
}
