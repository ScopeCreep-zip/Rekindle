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
