//! An attachment message is a channel post (plan E2.5): its `messages`
//! row is written `queued`, then it is a row of the outbox `channel_post`
//! stream of the record slot its write targets, settled by that write: a
//! stored page `sent`, a held one by the record pool's settle (the
//! desktop's delivery settle, as any post), one that cannot be made
//! `failed`. No send state is written outside the outbox.

use rekindle_codec::community::channel_record::ChannelMessage;

use crate::deps::{ChannelPost, ChannelWrite, FilesDeps};
use crate::error::FilesError;

/// Where an attachment message is written.
pub(crate) struct PostTarget<'a> {
    pub community_id: &'a str,
    pub channel_id: &'a str,
    pub record_key: &'a str,
    pub slot_index: u32,
    pub slot_keypair: &'a str,
    /// The slot's subkey (owner subkeys + `slot_index`).
    pub subkey: u32,
}

/// Queue `message`'s row and make its write; the write's outcome settles
/// the row.
///
/// # Errors
/// The row could not be stored, or the write could not be made (its row
/// fails).
pub(crate) async fn write_post<D: FilesDeps>(
    deps: &D,
    target: &PostTarget<'_>,
    message: &ChannelMessage,
    message_id: &str,
) -> Result<ChannelWrite, FilesError> {
    let post = ChannelPost {
        community: target.community_id.to_owned(),
        channel: target.channel_id.to_owned(),
        message: message_id.to_owned(),
        record_key: target.record_key.to_owned(),
        subkey: target.subkey,
    };
    deps.queue_post(&post).await?;
    let written = deps
        .write_channel_message_to_smpl(
            target.community_id,
            target.record_key,
            target.slot_index,
            target.slot_keypair,
            message,
        )
        .await;
    match written {
        Ok(ChannelWrite::Stored) => deps.settle_posts(target.record_key, target.subkey).await,
        Ok(ChannelWrite::Held) => {}
        Err(_) => deps.fail_post(message_id).await,
    }
    written
}

#[cfg(test)]
mod tests {
    use crate::deps::ChannelWrite;
    use crate::test_mock::MockDeps;
    use crate::upload::{upload_bytes_as_attachment, UploadRequest};

    async fn upload(deps: &MockDeps) {
        upload_bytes_as_attachment(
            deps,
            UploadRequest {
                community_id: "c1".into(),
                channel_id: "11111111111111111111111111111111".into(),
                bytes: b"payload".to_vec(),
                filename: "a.txt".into(),
                mime_type: "text/plain".into(),
                body_plaintext: Vec::new(),
                flags: 0,
                local_path: None,
            },
        )
        .await
        .expect("the upload");
    }

    #[tokio::test]
    async fn a_held_upload_stays_queued_until_the_pool_settles_it() {
        let mut deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(7, [4; 32]);
        deps.channel_write = ChannelWrite::Held;
        upload(&deps).await;
        let calls = deps.calls.lock();
        assert_eq!(calls.posts_queued.len(), 1, "a channel_post row");
        assert_eq!(calls.posts_queued[0].record_key, deps.channel_log_key);
        assert!(calls.posts_settled.is_empty(), "left to the pool's settle");
        assert!(calls.posts_failed.is_empty());
    }

    #[tokio::test]
    async fn a_stored_upload_settles_its_row_sent() {
        let deps = MockDeps::new("c1", "11111111111111111111111111111111").with_mek(7, [4; 32]);
        upload(&deps).await;
        let calls = deps.calls.lock();
        let queued = &calls.posts_queued[0];
        assert_eq!(
            calls.posts_settled,
            [(queued.record_key.clone(), queued.subkey)]
        );
    }
}
