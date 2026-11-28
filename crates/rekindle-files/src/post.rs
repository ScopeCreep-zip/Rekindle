//! An attachment message is a channel post (plan E3.3 slice 7): its
//! `messages` row is written `queued`, then it is a row of its channel's
//! outbox `channel_post` stream carrying the offer. The stream's sender
//! seals the carrying `ChannelMessage` at its `{anchor, channel, author,
//! streamSeq}` when it drains the row onto our channel outbox (`M`/4), and
//! settles the row from that append; the user's Retry queues the same row
//! again. No send state is written outside the outbox.

use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::message::PostEntry;

use crate::deps::{ChannelPost, FilesDeps};
use crate::error::FilesError;

/// Queue the attachment message `message_id` (its row already stored) on
/// `channel_id`'s stream with `offer` and `flags`.
///
/// # Errors
/// The row could not be queued.
pub(crate) async fn queue_attachment_post<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    offer: &AttachmentOffer,
    flags: u32,
) -> Result<(), FilesError> {
    let post = ChannelPost {
        governance_cut: Vec::new(),
        community: community_id.to_owned(),
        channel: channel_id.to_owned(),
        message: message_id.to_owned(),
        entry: PostEntry::Attachment {
            offer: offer.clone(),
            flags,
        },
    };
    deps.queue_post(&post).await
}

#[cfg(test)]
#[path = "post_tests.rs"]
mod tests;
