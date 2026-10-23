//! Plan E3.1 E3d: one member attaches a file to a post of the general
//! channel through the Host's `AttachmentUpload` (which writes the
//! carrying channel message), the other downloads it through
//! `AttachmentDownload`, served chunk by chunk over the uploader's route,
//! and the bytes it saves are the bytes that were attached.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, PinRequest};

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(240);

fn pins(request: PinRequest) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Pins(request))
}

/// Bytes that span several chunks and repeat no chunk.
fn file_bytes() -> Vec<u8> {
    (0..200_000u32)
        .map(|i| u8::try_from((i.wrapping_mul(2_654_435_761) >> 24) & 0xFF).unwrap_or(0))
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn attachment_round_trip() {
    let community = Community::of(2, 231).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let dir = tempfile::tempdir().expect("a folder");
    let source = dir.path().join("notes.bin");
    let bytes = file_bytes();
    std::fs::write(&source, &bytes).expect("the file to attach");

    let channel = community.general_id(0).await;
    let uploaded = community.nodes[0]
        .call(pins(PinRequest::AttachmentUpload {
            community: community.governance_key.clone(),
            channel_id: channel.clone(),
            path: source.display().to_string(),
        }))
        .await
        .expect("ann attaches the file");
    let attachment_id = uploaded.as_str().expect("an attachment id").to_owned();

    let target = dir.path().join("saved.bin");
    let reader = &community.nodes[1];
    let channel = community.general_id(1).await;
    let last = std::cell::RefCell::new(None);
    let saved = eventually(&community.sim, LIMIT, || async {
        let answer = reader
            .call(pins(PinRequest::AttachmentDownload {
                community: community.governance_key.clone(),
                channel_id: channel.clone(),
                attachment_id: attachment_id.clone(),
                path: target.display().to_string(),
            }))
            .await;
        let done = answer.is_ok();
        *last.borrow_mut() = answer.err();
        done
    })
    .await;
    assert!(saved, "bea downloads ann's attachment: {:?}", last.borrow());
    assert_eq!(
        std::fs::read(&target).expect("the saved file"),
        bytes,
        "the saved bytes are the attached bytes"
    );
}
