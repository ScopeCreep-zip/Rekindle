//! Expression upload limits on the Host (plan E3.1 E3c): the unlock opens
//! the chunk caches, and an oversized picture or one of another kind than
//! declared is refused before anything is cached or written.

use rekindle_channel::error::ChannelError;
use rekindle_channel::expressions::{MAX_STATIC_EMOJI_BYTES, MAX_STICKER_BYTES};
use rekindle_types::image_kind::ImageKind;

use super::{create_ann, offline_host};

/// A 1×1 RGBA PNG.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

#[tokio::test]
async fn uploads_over_the_limit_or_of_another_kind_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    host.ctx
        .files
        .ensure_open("c1")
        .expect("the unlock opened the caches");
    let channel = host.channel();

    let big = vec![0u8; MAX_STATIC_EMOJI_BYTES + 1];
    let refused = rekindle_channel::upload_emoji(&channel, "c1", "big", big, ImageKind::Png).await;
    assert!(matches!(refused, Err(ChannelError::BodyTooLarge { .. })));

    let big = vec![0u8; MAX_STICKER_BYTES + 1];
    let refused =
        rekindle_channel::upload_sticker(&channel, "c1", "big", big, ImageKind::Png, Vec::new())
            .await;
    assert!(matches!(refused, Err(ChannelError::BodyTooLarge { .. })));

    let refused =
        rekindle_channel::upload_emoji(&channel, "c1", "spark", PNG_1X1.to_vec(), ImageKind::Gif)
            .await;
    assert!(
        matches!(refused, Err(ChannelError::InvalidId(_))),
        "a PNG declared as a GIF is refused: {refused:?}"
    );
    assert!(host.ctx.files.holder_of(uuid::Uuid::nil()).is_none());
}
