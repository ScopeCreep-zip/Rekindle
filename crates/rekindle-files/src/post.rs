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
mod tests {
    use rekindle_types::message::PostEntry;

    use crate::test_mock::MockDeps;
    use crate::upload::{upload_bytes_as_attachment, UploadRequest};

    #[tokio::test]
    async fn an_upload_queues_its_message_on_the_channels_outbox_stream() {
        let deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(7, [4; 32]);
        let id = upload_bytes_as_attachment(
            &deps,
            UploadRequest {
                community_id: "c1".into(),
                channel_id: "11111111111111111111111111111111".into(),
                bytes: b"payload".to_vec(),
                filename: "a.txt".into(),
                mime_type: "text/plain".into(),
                body_plaintext: Vec::new(),
                flags: rekindle_types::channel::flags::VOICE_MESSAGE,
                local_path: None,
            },
        )
        .await
        .expect("the upload");
        let calls = deps.calls.lock();
        assert_eq!(calls.posts_queued.len(), 1, "one channel_post row");
        let post = &calls.posts_queued[0];
        assert_eq!(post.scope(), "c1:11111111111111111111111111111111");
        let PostEntry::Attachment { offer, flags } = &post.entry else {
            panic!("an attachment post carries its offer");
        };
        assert_eq!(hex::encode(offer.attachment_id), id);
        assert_eq!(*flags, rekindle_types::channel::flags::VOICE_MESSAGE);
    }
}
